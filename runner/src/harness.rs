//! Spawns the `zeta` server binary as a child process exposing the MySQL
//! wire endpoint, and tears it down on Drop.
//!
//! The harness deliberately spawns the binary rather than linking the zeta
//! crates as a library — this preserves the licensing wall (no zeta-internal
//! types in this Apache repo) and lets any zeta build be tested without
//! rebuilding the runner.

use anyhow::{anyhow, Context, Result};
use regex::Regex;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::time::timeout;

const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// A running `zeta` child process with a MySQL wire listener bound on an
/// ephemeral local port.
pub struct ZetaServer {
    child: Option<Child>,
    port: u16,
    _data_dir: TempDir,
}

impl ZetaServer {
    pub async fn start(zeta_bin: &Path) -> Result<Self> {
        if !zeta_bin.exists() {
            return Err(anyhow!(
                "zeta binary not found at {} — pass --zeta-bin pointing at a built `zeta` server binary",
                zeta_bin.display()
            ));
        }

        let port = pick_free_port()?;
        let data_dir = tempfile::Builder::new()
            .prefix("zeta-mysql-compat-")
            .tempdir()
            .context("creating temp data dir")?;

        let mut cmd = Command::new(zeta_bin);
        cmd.arg("--no-pg")
            .arg("--bind")
            .arg("127.0.0.1")
            .arg("--mysql-port")
            .arg(port.to_string())
            .arg("--data-dir")
            .arg(data_dir.path())
            .arg("--storage-backend")
            .arg("memory")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        cmd.env(
            "RUST_LOG",
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()),
        );

        let mut child = cmd.spawn().context("spawning zeta")?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("child stderr not captured"))?;

        match timeout(READY_TIMEOUT, wait_for_ready(stderr, port)).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = child.kill().await;
                return Err(e);
            }
            Err(_) => {
                let _ = child.kill().await;
                return Err(anyhow!(
                    "timed out after {:?} waiting for zeta MySQL listener on port {port}",
                    READY_TIMEOUT
                ));
            }
        }

        Ok(ZetaServer {
            child: Some(child),
            port,
            _data_dir: data_dir,
        })
    }

    #[allow(dead_code)]
    pub fn mysql_port(&self) -> u16 {
        self.port
    }

    pub fn mysql_url(&self, database: &str) -> String {
        format!("mysql://root@127.0.0.1:{}/{}", self.port, database)
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        if let Some(pid) = child.id() {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            );
        }
        match timeout(Duration::from_secs(10), child.wait()).await {
            Ok(_) => Ok(()),
            Err(_) => {
                let _ = child.kill().await;
                Ok(())
            }
        }
    }
}

impl Drop for ZetaServer {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            if let Some(pid) = child.id() {
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid as i32),
                    nix::sys::signal::Signal::SIGTERM,
                );
            }
            // kill_on_drop(true) reaps the child; we just nudged it to exit
            // gracefully via SIGTERM first.
        }
    }
}

fn pick_free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").context("binding ephemeral port")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

async fn wait_for_ready(stderr: tokio::process::ChildStderr, expected_port: u16) -> Result<()> {
    // Banner from zeta-server-bin: "Zeta MySQL wire listener ready on <bind>:<port> ..."
    let pat = Regex::new(r"Zeta MySQL wire listener ready on \S+:(\d+)").expect("compile regex");

    let mut lines = BufReader::new(stderr).lines();
    while let Some(line) = lines.next_line().await? {
        eprintln!("[zeta] {}", line);
        if let Some(caps) = pat.captures(&line) {
            let reported: u16 = caps[1].parse().unwrap_or(0);
            if reported == expected_port {
                return Ok(());
            }
            return Err(anyhow!(
                "zeta listener bound on unexpected port {reported} (expected {expected_port})"
            ));
        }
    }
    Err(anyhow!("zeta exited before MySQL listener became ready"))
}
