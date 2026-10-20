//! Walks `tests/<suite>/**/*.slt` and runs each file against a live zeta
//! MySQL endpoint via `sqllogictest::Runner` + a `mysql_async`-backed
//! `AsyncDB`.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use mysql_async::prelude::Queryable;
use sqllogictest::{AsyncDB, DBOutput, DefaultColumnType};
use walkdir::WalkDir;

const TESTS_ROOT: &str = "tests";

/// Run every `.slt` under `tests/<suite>/` (recursive). `suite == "all"`
/// runs every suite directory present under `tests/`.
pub async fn run_suite(suite: &str, mysql_url: &str, filter: Option<&str>) -> Result<()> {
    let files = discover_slt_files(suite, filter)?;
    if files.is_empty() {
        eprintln!(
            "no .slt files matched suite={suite}{}",
            filter.map(|f| format!(" filter={f}")).unwrap_or_default()
        );
        return Ok(());
    }

    let url = mysql_url.to_string();
    eprintln!("connecting via {}", url);
    let opts = session_opts(&url)?;

    let make_conn = move || {
        let opts = opts.clone();
        async move { ZetaConn::connect(opts, "zeta-mysql").await }
    };

    let mut passed = 0usize;
    let mut failed: Vec<(PathBuf, String)> = Vec::new();
    for file in &files {
        eprintln!("→ {}", file.display());
        let mut runner = sqllogictest::Runner::new(make_conn.clone());
        match runner.run_file_async(file).await {
            Ok(()) => {
                passed += 1;
                eprintln!("  OK");
            }
            Err(e) => {
                let msg = format!("{}", e.display(false));
                eprintln!("  FAIL\n{}", msg);
                failed.push((file.clone(), msg));
            }
        }
    }

    eprintln!("\n{passed} passed, {} failed", failed.len());
    if !failed.is_empty() {
        return Err(anyhow!("{} .slt file(s) failed", failed.len()));
    }
    Ok(())
}

const SYSTEM_DBS: [&str; 4] = ["information_schema", "mysql", "performance_schema", "sys"];

/// Re-derive the expectations of every matching `.slt` file from a real MySQL server (the oracle): each file is run
/// against it and its `query` results, and which statements succeed or fail (and with which error number), are
/// rewritten in place to what MySQL did. The oracle's non-system databases are DROPPED before each file, so it must
/// be a throwaway server on this machine (anything else is refused unless `allow_remote`).
pub async fn update_from_oracle(
    oracle_base: &str,
    allow_remote: bool,
    mark_extensions: bool,
    suite: &str,
    filter: Option<&str>,
) -> Result<()> {
    let base_opts = session_opts(oracle_base)?;
    let host = base_opts.ip_or_hostname().to_string();
    if !allow_remote && !matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1" | "[::1]") {
        return Err(anyhow!(
            "oracle host `{host}` is not this machine, and the run DROPS every non-system database on it; \
             pass --allow-remote-oracle if that is really what you want"
        ));
    }
    // The URL names no database (a path in it would put the administrative session in that database).
    let test_opts: mysql_async::Opts = mysql_async::OptsBuilder::from_opts(base_opts.clone())
        .db_name(Some("test"))
        .into();
    let admin_opts: mysql_async::Opts = mysql_async::OptsBuilder::from_opts(base_opts)
        .db_name(None::<String>)
        .into();
    let files = discover_slt_files(suite, filter)?;
    if files.is_empty() {
        return Err(anyhow!("no .slt files matched; nothing to update"));
    }
    let mut changed = 0usize;
    let mut flips = 0usize;
    for (n, file) in files.iter().enumerate() {
        // Fresh server state: every database but the system ones dropped, `test` recreated.
        let mut admin = mysql_async::Conn::new(admin_opts.clone()).await?;
        if n == 0 {
            let (version, tz): (String, String) = admin
                .query_first("SELECT VERSION(), @@time_zone")
                .await?
                .ok_or_else(|| anyhow!("no answer to SELECT VERSION()"))?;
            eprintln!("oracle: MySQL {version}, time_zone {tz}");
            if !tz.contains("00:00") && tz != "UTC" {
                return Err(anyhow!(
                    "the oracle's time_zone is `{tz}`: start it with --default-time-zone=+00:00 (zeta evaluates in UTC)"
                ));
            }
        }
        let dbs: Vec<String> = admin.query("SHOW DATABASES").await?;
        for db in dbs {
            if !SYSTEM_DBS.contains(&db.as_str()) {
                admin
                    .query_drop(format!("DROP DATABASE `{}`", db.replace('`', "``")))
                    .await?;
            }
        }
        admin.query_drop("CREATE DATABASE test").await?;
        admin.disconnect().await?;

        // A bare `statement error` accepts any error, so it would never learn which one MySQL gives: make it
        // fail to match first, and the update then records the real number (or turns it into `statement ok`).
        // The update works on a copy, so the file itself is only ever replaced by a finished result: an interrupted
        // run leaves it as it was.
        let before = std::fs::read_to_string(file)?;
        let work = file.with_extension("slt.primed");
        let work_tmp = file.with_extension("slt.primed.temp");
        std::fs::write(&work, prime_bare_errors(&before))?;

        eprintln!("→ {}", file.display());
        DRIVER_ERRORS.store(0, std::sync::atomic::Ordering::SeqCst);
        let make_conn = {
            let opts = test_opts.clone();
            move || {
                let opts = opts.clone();
                async move { ZetaConn::connect(opts, "mysql").await }
            }
        };
        let mut runner = sqllogictest::Runner::new(make_conn);
        let res = runner
            .update_test_file(
                &work,
                " ",
                sqllogictest::default_validator,
                sqllogictest::default_column_validator,
            )
            .await;
        let cleanup = || {
            let _ = std::fs::remove_file(&work);
            let _ = std::fs::remove_file(&work_tmp);
        };
        if let Err(e) = res {
            cleanup();
            return Err(anyhow!("updating {}: {e}", file.display()));
        }
        let driver = DRIVER_ERRORS.load(std::sync::atomic::Ordering::SeqCst);
        if driver > 0 {
            cleanup();
            return Err(anyhow!(
                "{driver} connection or protocol errors while updating {}: not writing anything",
                file.display()
            ));
        }
        let updated = std::fs::read_to_string(&work)?;
        cleanup();

        // The update never adds or removes records, so the old and new texts line up block for block. If they do not
        // (a result cell with a blank line in it, say), nothing below can be trusted.
        let (old_text, new_text) = (trim_end_nl(&before), trim_end_nl(&updated));
        let (ob, nb) = (blocks(&old_text), blocks(&new_text));
        if ob.len() != nb.len() {
            return Err(anyhow!(
                "{}: the updated file has {} blocks, the original {}; refusing to write it",
                file.display(),
                nb.len(),
                ob.len()
            ));
        }
        // Records the oracle did not run (`onlyif zeta-mysql`, `skipif mysql`) keep their primed expectation: undo it.
        // A primed record that WAS run and still expects `ERROR 0` is a statement MySQL accepted but sqllogictest
        // cannot rewrite (an error expectation over a SELECT-like statement): say so instead of writing a lie.
        let new_text = unprime(&new_text).map_err(|e| anyhow!("{}: {e}", file.display()))?;
        // A record that succeeded for zeta's pin but fails on MySQL is either a zeta extension (PostgreSQL syntax,
        // extra functions: MySQL has no answer to pin it to) or a validation MySQL does and zeta does not. Only a
        // person can tell which. By default the oracle's answer is written, and the record is listed; with
        // --mark-zeta-extensions it is put back under `onlyif zeta-mysql` (run for zeta only, pinned to zeta's own
        // answer). A record already marked `onlyif zeta-mysql` is never touched by an update.
        for line in flipped_records(&old_text, &new_text) {
            flips += 1;
            eprintln!("  success -> MySQL error: {line}");
        }
        let restored = if mark_extensions {
            keep_zeta_extensions(&old_text, &new_text)
        } else {
            new_text
        };
        let restored = format!("{}\n", restored.trim_end_matches('\n'));
        if restored != format!("{}\n", before.trim_end_matches('\n')) {
            std::fs::write(file, &restored)?;
            changed += 1;
        }
    }
    eprintln!("{} files, {changed} changed", files.len());
    if flips > 0 && !mark_extensions {
        eprintln!(
            "{flips} records went from success to a MySQL error and were rewritten to the error. If one of them is a zeta extension (PostgreSQL syntax, an extra function) rather than a validation zeta lacks, put `onlyif zeta-mysql` on the line above it and restore its old expectation."
        );
    }
    Ok(())
}

fn trim_end_nl(s: &str) -> String {
    format!("{}\n", s.trim_end_matches('\n'))
}

/// Undo the priming (`ERROR 0`) on records that carry a condition and so were never run against the oracle; fail on
/// a primed record that was run and not rewritten.
fn unprime(text: &str) -> std::result::Result<String, String> {
    let mut out = String::with_capacity(text.len());
    for block in blocks(text) {
        let mut b = block;
        let primed = b.lines().any(|l| {
            let t = l.trim_end();
            t == "statement error ERROR 0" || t == "query error ERROR 0"
        });
        if primed {
            let conditional = b
                .lines()
                .any(|l| l.starts_with("onlyif ") || l.starts_with("skipif "));
            if !conditional {
                let stmt = b
                    .lines()
                    .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                    .nth(1)
                    .unwrap_or("");
                return Err(format!(
                    "MySQL accepted `{}` but its expectation is an error that sqllogictest cannot rewrite; edit the record by hand",
                    stmt.chars().take(100).collect::<String>()
                ));
            }
            b = b
                .replace("statement error ERROR 0", "statement error")
                .replace("query error ERROR 0", "query error");
        }
        out.push_str(&b);
    }
    Ok(out)
}

/// Is this block a record whose expectation is an error?
fn is_error_record(block: &str) -> bool {
    block
        .lines()
        .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .is_some_and(|l| l.starts_with("statement error") || l.starts_with("query error"))
}

/// Is this block an executable record (statement or query)?
fn is_record(block: &str) -> bool {
    block
        .lines()
        .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .is_some_and(|l| l.starts_with("statement") || l.starts_with("query"))
}

/// Blocks of a `.slt` file: runs of lines separated by blank lines, with the separators kept.
fn blocks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for line in text.split_inclusive('\n') {
        if line.trim().is_empty() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(line.to_string());
        } else {
            cur.push_str(line);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// First line of the statement of every record that was a success before the update and is an error after it.
fn flipped_records(old: &str, new: &str) -> Vec<String> {
    let (ob, nb) = (blocks(old), blocks(new));
    if ob.len() != nb.len() {
        return Vec::new();
    }
    ob.iter()
        .zip(&nb)
        .filter(|(o, n)| is_record(o) && !is_error_record(o) && is_error_record(n))
        .map(|(o, _)| {
            o.lines()
                .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                .nth(1)
                .unwrap_or("")
                .chars()
                .take(100)
                .collect()
        })
        .collect()
}

/// Put back, under `onlyif zeta-mysql`, every record that was a success before the update and is an error after it.
/// The update never adds or removes records, so the old and new blocks line up one to one.
fn keep_zeta_extensions(old: &str, new: &str) -> String {
    let (ob, nb) = (blocks(old), blocks(new));
    if ob.len() != nb.len() {
        return new.to_string();
    }
    let mut out = String::with_capacity(new.len());
    for (o, n) in ob.iter().zip(&nb) {
        if is_record(o) && !is_error_record(o) && is_error_record(n) {
            // Keep the comment lines of the old block, then the condition, then the old record.
            let mut put = false;
            for line in o.split_inclusive('\n') {
                if !put && !line.trim().is_empty() && !line.trim_start().starts_with('#') {
                    out.push_str("onlyif zeta-mysql\n");
                    put = true;
                }
                out.push_str(line);
            }
        } else {
            out.push_str(n);
        }
    }
    out
}

/// `statement error` / `query error` with nothing after it expects "any error". Give it an expectation that no
/// real error number matches, so that updating from a server rewrites it to that server's error number.
fn prime_bare_errors(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let t = line.trim_end();
        if t == "statement error" || t == "query error" {
            out.push_str(t);
            out.push_str(" ERROR 0\n");
        } else {
            out.push_str(line);
        }
    }
    out
}

fn discover_slt_files(suite: &str, filter: Option<&str>) -> Result<Vec<PathBuf>> {
    let root: PathBuf = if suite == "all" {
        PathBuf::from(TESTS_ROOT)
    } else {
        PathBuf::from(TESTS_ROOT).join(suite)
    };
    if !root.exists() {
        return Err(anyhow!(
            "suite directory {} does not exist (cwd={})",
            root.display(),
            std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        ));
    }

    let mut out = Vec::new();
    for entry in WalkDir::new(&root).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.into_path();
        if path.extension().and_then(|s| s.to_str()) != Some("slt") {
            continue;
        }
        if let Some(needle) = filter {
            if !path.to_string_lossy().contains(needle) {
                continue;
            }
        }
        out.push(path);
    }
    out.sort();
    Ok(out)
}

/// MySQL-wire-protocol AsyncDB backed by a single mysql_async connection.
struct ZetaConn {
    conn: mysql_async::Conn,
    /// What `onlyif` / `skipif` conditions see: `zeta-mysql` for zeta, `mysql` for the oracle. A record marked
    /// `onlyif zeta-mysql` is a zeta extension that MySQL does not have: it is run (and pinned) for zeta only.
    engine: &'static str,
}

/// Connection options for a session of this runner: no unix-socket upgrade (it would talk to whatever local server
/// owns the path the server reports, not the one that was asked for), and `SET NAMES utf8mb4`, which gives the
/// server's default collation (MySQL 8: utf8mb4_0900_ai_ci, NO PAD) as JDBC, mysql2 and the `mysql` client get it,
/// instead of the library handshake's utf8mb4_general_ci (PAD SPACE).
fn session_opts(url: &str) -> Result<mysql_async::Opts> {
    let o = mysql_async::Opts::from_url(url).map_err(|e| anyhow!("bad URL `{url}`: {e}"))?;
    Ok(mysql_async::OptsBuilder::from_opts(o)
        .prefer_socket(false)
        .init(vec!["SET NAMES utf8mb4"])
        .into())
}

/// Non-server errors (a lost connection, a protocol error) seen by any session. The update mode must not write
/// them into an expectation as if MySQL had said them.
static DRIVER_ERRORS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl ZetaConn {
    async fn connect(opts: mysql_async::Opts, engine: &'static str) -> Result<Self, ZetaConnError> {
        let conn = mysql_async::Conn::new(opts)
            .await
            .map_err(|e| ZetaConnError::msg(e.to_string()))?;
        Ok(Self { conn, engine })
    }
}

#[async_trait]
impl AsyncDB for ZetaConn {
    type Error = ZetaConnError;
    type ColumnType = DefaultColumnType;

    async fn run(&mut self, sql: &str) -> Result<DBOutput<Self::ColumnType>, Self::Error> {
        let kind = leading_keyword(sql);
        let returns_rows = matches!(
            kind.as_deref(),
            Some("SELECT")
                | Some("SHOW")
                | Some("EXPLAIN")
                | Some("DESCRIBE")
                | Some("DESC")
                | Some("WITH")
                | Some("VALUES")
                | Some("TABLE")
        );

        if returns_rows {
            let rows: Vec<mysql_async::Row> = self.conn.query(sql).await.map_err(wire_error)?;
            let types: Vec<DefaultColumnType> = rows
                .first()
                .map(|r| {
                    r.columns_ref()
                        .iter()
                        .map(|_| DefaultColumnType::Any)
                        .collect()
                })
                .unwrap_or_default();
            let mut out_rows: Vec<Vec<String>> = Vec::with_capacity(rows.len());
            for row in rows {
                let mut cells = Vec::with_capacity(row.len());
                for i in 0..row.len() {
                    let v: &mysql_async::Value = row
                        .as_ref(i)
                        .ok_or_else(|| ZetaConnError::msg("missing column"))?;
                    cells.push(format_value(v));
                }
                out_rows.push(cells);
            }
            Ok(DBOutput::Rows {
                types,
                rows: out_rows,
            })
        } else {
            self.conn.query_drop(sql).await.map_err(wire_error)?;
            Ok(DBOutput::StatementComplete(self.conn.affected_rows()))
        }
    }

    fn engine_name(&self) -> &str {
        self.engine
    }

    async fn sleep(dur: Duration) {
        tokio::time::sleep(dur).await;
    }
}

/// A server error as the expectations see it: just `ERROR <number>`. `statement error ERROR 1146` then means
/// "MySQL error 1146", whatever each server words its message like; the message text is not part of the contract.
/// Anything that is not a server error (connection lost, protocol error) keeps its own text.
fn wire_error(e: mysql_async::Error) -> ZetaConnError {
    match e {
        mysql_async::Error::Server(se) => ZetaConnError::msg(format!("ERROR {}", se.code)),
        other => {
            DRIVER_ERRORS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            ZetaConnError::msg(other.to_string())
        }
    }
}

fn leading_keyword(sql: &str) -> Option<String> {
    let s = sql.trim_start();
    // Skip line comments.
    let s = s.lines().find(|l| !l.trim().starts_with("--"))?;
    let word = s.split(|c: char| c.is_whitespace() || c == '(').next()?;
    Some(word.to_ascii_uppercase())
}

fn format_value(v: &mysql_async::Value) -> String {
    use mysql_async::Value::*;
    match v {
        NULL => "NULL".to_string(),
        Int(i) => i.to_string(),
        UInt(u) => u.to_string(),
        Float(f) => f.to_string(),
        Double(d) => d.to_string(),
        Bytes(b) => match std::str::from_utf8(b) {
            Ok(s) if s.is_empty() => "(empty)".to_string(),
            Ok(s) => s.to_string(),
            Err(_) => format!("0x{}", hex_lower(b)),
        },
        Date(y, m, d, hh, mm, ss, _us) => {
            if *hh == 0 && *mm == 0 && *ss == 0 {
                format!("{y:04}-{m:02}-{d:02}")
            } else {
                format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
            }
        }
        Time(neg, days, h, m, s, _us) => {
            let sign = if *neg { "-" } else { "" };
            let total_h = (*days as u32) * 24 + (*h as u32);
            format!("{sign}{total_h:02}:{m:02}:{s:02}")
        }
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[derive(Debug)]
pub struct ZetaConnError(String);

impl ZetaConnError {
    fn msg(s: impl Into<String>) -> Self {
        ZetaConnError(s.into())
    }
}

impl std::fmt::Display for ZetaConnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ZetaConnError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_error_expectations_are_primed() {
        assert_eq!(
            prime_bare_errors("statement error\nSELECT 1\n\nstatement error ERROR 1\nx\n"),
            "statement error ERROR 0\nSELECT 1\n\nstatement error ERROR 1\nx\n"
        );
    }

    #[test]
    fn a_success_that_became_an_error_is_kept_as_a_zeta_extension() {
        let old = "# note\nstatement ok\nCREATE TABLE t (a JSONB)\n\nquery I\nSELECT 1\n----\n1\n\nstatement error\nDROP TABLE nx\n";
        let new = "# note\nstatement error ERROR 1064\nCREATE TABLE t (a JSONB)\n\nquery I\nSELECT 1\n----\n2\n\nstatement error ERROR 1051\nDROP TABLE nx\n";
        assert_eq!(
            keep_zeta_extensions(old, new),
            "# note\nonlyif zeta-mysql\nstatement ok\nCREATE TABLE t (a JSONB)\n\nquery I\nSELECT 1\n----\n2\n\nstatement error ERROR 1051\nDROP TABLE nx\n"
        );
    }

    #[test]
    fn priming_is_undone_only_where_the_oracle_never_ran_the_record() {
        let skipped = "onlyif zeta-mysql\nstatement error ERROR 0\nSELECT 1\n\n";
        assert_eq!(
            unprime(skipped).unwrap(),
            "onlyif zeta-mysql\nstatement error\nSELECT 1\n\n"
        );
        // Run by the oracle and still primed: MySQL accepted a statement that the file says fails.
        let err = unprime("statement error ERROR 0\nSELECT 1\n").unwrap_err();
        assert!(err.contains("SELECT 1"), "{err}");
        // A learned error number is not touched.
        assert_eq!(
            unprime("statement error ERROR 1062\nINSERT x\n").unwrap(),
            "statement error ERROR 1062\nINSERT x\n"
        );
    }
}
