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

## License

Apache 2.0. See `LICENSE`. Attribution for upstream-derived material in `NOTICE`.
