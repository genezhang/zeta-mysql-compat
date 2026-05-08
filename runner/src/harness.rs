#![allow(dead_code)] // skeleton — fields/methods land as the runner is fleshed out

use anyhow::Result;
use std::path::Path;

/// Spawns the `zeta` server binary on an ephemeral TCP port and exposes the
/// MySQL wire endpoint to test runners. On drop, signals shutdown and waits
/// for the child to exit.
pub struct ZetaServer {
    // TODO: child process handle, port, tempdir
}

impl ZetaServer {
    pub async fn start(_zeta_bin: &Path) -> Result<Self> {
        // TODO: spawn `zeta --bind 127.0.0.1 --port 0 --data-dir <tmp>` with
        // MySQL listener flags, scrape the chosen port from stderr or a
        // /healthz endpoint, return once the server accepts connections.
        anyhow::bail!("ZetaServer::start: not yet implemented")
    }

    pub fn mysql_port(&self) -> u16 {
        unimplemented!()
    }
}
