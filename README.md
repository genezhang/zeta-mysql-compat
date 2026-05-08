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

Skeleton. Runner stub builds; suites are empty. Will be populated as Zeta's MySQL surface and binlog implementation land.

## Running

```
cargo run --release -- --zeta-bin <path-to-zeta-binary> --suite all
```

The runner spawns the supplied `zeta` binary on an ephemeral port and drives test suites against its MySQL wire protocol.

## License

Apache 2.0. See `LICENSE`. Attribution for upstream-derived material in `NOTICE`.
