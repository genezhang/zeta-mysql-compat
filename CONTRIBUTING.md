# Contributing to zeta-mysql-compat

## Licensing wall (read before contributing test material)

This repo is **Apache 2.0**. Its sibling, [`zeta-mysql-compat-mtr`](https://github.com/genezhang/zeta-mysql-compat-mtr), is **GPL v2**. The wall between them exists to keep the licenses from contaminating each other.

### Hard rules

1. **No MTR-derived material in this repo. Ever.**
   If a test was *adapted, ported, lightly modified, or transcribed* from MySQL's `mysql-test/` or MariaDB's `mysql-test/` directory — even if it now reads as generic SQL — it is GPL-licensed by lineage. It belongs in `zeta-mysql-compat-mtr`, not here.

2. **Lineage matters more than current text.** A test derived from MTR but rewritten in `.slt` format is still GPL-derivative. Don't try to "convert" tests across the wall.

3. **No code dependencies on the GPL repo.** Don't import its runner, helpers, or fixtures via Cargo or any other mechanism. Re-derive independently if you need similar functionality.

4. **Acceptable upstream sources for tests in this repo:**
   - Zeta-authored (original work)
   - Dolt's `go-mysql-server` enginetest (Apache 2.0)
   - TiDB integration tests (Apache 2.0)
   - RisingWave `e2e_test` (Apache 2.0)
   - Other permissively-licensed (Apache 2.0 / MIT / BSD / Public Domain) sources, with attribution in `NOTICE` and a per-directory `ATTRIBUTION.md`.

5. **When in doubt, ask.** Open an issue rather than guessing the license is OK.

## Test format

Zeta-authored tests use [`sqllogictest`](https://github.com/risinglightdb/sqllogictest-rs) `.slt` format. Tests live under `tests/<source>/` organized by topic.

## Running locally

```
cargo run --release -- --zeta-bin /path/to/zeta --suite zeta
```

## CI

Each PR runs the full suite against a fresh-built zeta binary (downloaded as an artifact from the [zeta repo](https://github.com/genezhang/zeta)).

## Skip-list

Known-failing tests are listed in `tests/skip_list.toml` with reason categories:

- `bug` — confirmed Zeta defect, with a tracking issue in [zeta](https://github.com/genezhang/zeta/issues)
- `not-yet-implemented` — feature isn't built yet, scheduled
- `intentional-divergence` — Zeta deliberately diverges from MySQL behavior, documented

A passing test that starts failing without an entry is a regression.
