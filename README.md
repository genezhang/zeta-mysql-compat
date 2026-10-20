# zeta-mysql-compat

MySQL wire-protocol compatibility test suite for [Zeta](https://github.com/genezhang/zeta), a distributed database with PostgreSQL- and MySQL-compatible wire protocols.

This repo holds the **permissively-licensed** half of Zeta's MySQL-compat test material:

- Zeta-authored regressions (`tests/zeta/`)
- Ports of Dolt's [`go-mysql-server`](https://github.com/dolthub/go-mysql-server) `enginetest` queries (`tests/dolt/`, Apache 2.0 upstream)
- Ports of TiDB integration tests (`tests/tidb/`, Apache 2.0 upstream)
- Mirrors of RisingWave's `e2e_test` (`tests/risingwave/`, Apache 2.0 upstream)
- Docker-based binlog E2E harness with Debezium (`tests/binlog-e2e/`)

GPL-licensed material (MySQL MTR-derived `.test`/`.result` files and the MTR DSL runner) lives in a **separate repo**, [`zeta-mysql-compat-mtr`](https://github.com/genezhang/zeta-mysql-compat-mtr), with no shared code or dependencies. See `CONTRIBUTING.md` for the licensing wall.

## Status

**M0 — first suites green.** Runner spawns zeta, drives `.slt` files via `sqllogictest` over the MySQL wire, and reports per-file pass/fail. `tests/zeta/select_basic.slt` and `tests/zeta/ddl_crud.slt` pass against the current `zeta` binary. Other suites (`dolt/`, `tidb/`, `risingwave/`, `binlog-e2e/`) are still empty and will be populated as Zeta's MySQL surface and binlog implementation land.

## Running

Build a `zeta` binary in the main repo (`cargo build -p zeta-server-bin`), then:

```
cd runner
cargo run -- --zeta-bin /path/to/zeta --suite zeta
```

`--suite all` walks every directory under `tests/`. `--filter <substring>` limits to matching `.slt` paths. The runner picks a free port, spawns `zeta --no-pg --bind 127.0.0.1 --mysql-port <port> --storage-backend memory`, waits for the listener-ready banner, then runs each `.slt` file with a fresh `mysql_async` connection.

## Where the expectations come from

The expected results in `tests/zeta/*.slt` are what a real MySQL 8.4.6 answers, not what zeta printed when the test was written: query results, whether each statement succeeds or fails, and the error number of a failure (`statement error ERROR 1062`; the message text is not compared). Re-derive them with a throwaway MySQL (`--default-time-zone=+00:00`, default `sql_mode`; **its non-system databases are dropped**):

```
cargo run --release -- --update-from-oracle mysql://root@127.0.0.1:33306
```

The files are rewritten in place and a second run changes nothing. A record that succeeded before and now fails on MySQL is listed: if it is a zeta extension (PostgreSQL syntax, a function MySQL does not have) put `onlyif zeta-mysql` on the line above it and restore its old expectation, so it runs for zeta only and is visibly a zeta-only pin; if it is a validation zeta lacks, keep MySQL's answer. `--mark-zeta-extensions` does the first for every such record, for converting a file once. `--zeta-bin` is not needed in this mode.

## License

Apache 2.0. See `LICENSE`. Attribution for upstream-derived material in `NOTICE`.
