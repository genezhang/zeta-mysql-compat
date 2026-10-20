use anyhow::Result;
use clap::Parser;
use mysql_async::prelude::Queryable;
use std::path::PathBuf;

mod harness;
mod slt_runner;

#[derive(Parser, Debug)]
#[command(version, about = "MySQL-compat test runner for Zeta")]
struct Args {
    /// Path to a built `zeta` server binary (from crates/zeta-server-bin in
    /// the main zeta repo). Not needed with `--update-from-oracle`.
    #[arg(long)]
    zeta_bin: Option<PathBuf>,

    /// Test suite to run. Use `all` for every suite under tests/.
    #[arg(long, default_value = "all")]
    suite: String,

    /// Optional substring filter applied to the .slt path.
    #[arg(long)]
    filter: Option<String>,

    /// Re-derive the expectations of the matching `.slt` files from a real MySQL server, e.g.
    /// `mysql://root@127.0.0.1:33306`, instead of testing zeta: query results, success or failure of each
    /// statement, and error numbers are rewritten in place to what MySQL did. THE SERVER'S NON-SYSTEM DATABASES
    /// ARE DROPPED; it must be a throwaway server (MySQL 8.4, `--default-time-zone=+00:00`).
    #[arg(long, value_name = "URL")]
    update_from_oracle: Option<String>,

    /// With `--update-from-oracle`: a record that succeeded before and now fails on MySQL is put back under
    /// `onlyif zeta-mysql` with its old expectation (for zeta extensions such as PostgreSQL syntax) instead of
    /// being rewritten to the error. Use it once when converting a file; later updates leave marked records alone.
    #[arg(long)]
    mark_zeta_extensions: bool,

    /// With `--update-from-oracle`: allow a server that is not on this machine.
    #[arg(long)]
    allow_remote_oracle: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    if let Some(url) = &args.update_from_oracle {
        return slt_runner::update_from_oracle(
            url,
            args.allow_remote_oracle,
            args.mark_zeta_extensions,
            &args.suite,
            args.filter.as_deref(),
        )
        .await;
    }
    let Some(zeta_bin) = &args.zeta_bin else {
        anyhow::bail!("--zeta-bin is required (only --update-from-oracle runs without zeta)");
    };

    let mut zeta = harness::ZetaServer::start(zeta_bin).await?;
    // `test` is not pre-created by the server: `USE`/a handshake naming a missing database is
    // error 1049, as in MySQL. Create it here, over a connection that names no database.
    let mut admin =
        mysql_async::Conn::new(mysql_async::Opts::from_url(&zeta.mysql_url(""))?).await?;
    admin
        .query_drop("CREATE DATABASE IF NOT EXISTS test")
        .await?;
    admin.disconnect().await?;
    let url = zeta.mysql_url("test");
    let result = slt_runner::run_suite(&args.suite, &url, args.filter.as_deref()).await;
    let _ = zeta.shutdown().await;
    result
}
