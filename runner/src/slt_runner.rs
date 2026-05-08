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

    let make_conn = move || {
        let url = url.clone();
        async move { ZetaConn::connect(&url).await }
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
}

impl ZetaConn {
    async fn connect(url: &str) -> Result<Self, ZetaConnError> {
        let base =
            mysql_async::Opts::from_url(url).map_err(|e| ZetaConnError::msg(e.to_string()))?;
        // Pre-populate session settings so mysql_async skips the post-handshake
        // `SELECT @@socket, @@max_allowed_packet, @@wait_timeout` probe — zeta's
        // MySQL wire doesn't return these as integers and the probe panics in
        // mysql_common's value conversion.
        let opts: mysql_async::Opts = mysql_async::OptsBuilder::from_opts(base)
            .prefer_socket(false)
            .max_allowed_packet(Some(16 * 1024 * 1024))
            .wait_timeout(Some(28800))
            .into();
        let conn = mysql_async::Conn::new(opts)
            .await
            .map_err(|e| ZetaConnError::msg(e.to_string()))?;
        Ok(Self { conn })
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
            let rows: Vec<mysql_async::Row> = self
                .conn
                .query(sql)
                .await
                .map_err(|e| ZetaConnError::msg(e.to_string()))?;
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
            self.conn
                .query_drop(sql)
                .await
                .map_err(|e| ZetaConnError::msg(e.to_string()))?;
            Ok(DBOutput::StatementComplete(self.conn.affected_rows()))
        }
    }

    fn engine_name(&self) -> &str {
        "zeta-mysql"
    }

    async fn sleep(dur: Duration) {
        tokio::time::sleep(dur).await;
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
