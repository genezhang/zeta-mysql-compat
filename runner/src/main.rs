use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

mod harness;
mod slt_runner;

#[derive(Parser, Debug)]
#[command(version, about = "MySQL-compat test runner for Zeta")]
struct Args {
    /// Path to a built `zeta` server binary (from crates/zeta-server-bin in
    /// the main zeta repo).
    #[arg(long)]
    zeta_bin: PathBuf,

    /// Test suite to run. Use `all` for every suite under tests/.
    #[arg(long, default_value = "all")]
    suite: String,

    /// Optional substring filter applied to the .slt path.
    #[arg(long)]
    filter: Option<String>,
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

    let mut zeta = harness::ZetaServer::start(&args.zeta_bin).await?;
    let url = zeta.mysql_url("test");
    let result = slt_runner::run_suite(&args.suite, &url, args.filter.as_deref()).await;
    let _ = zeta.shutdown().await;
    result
}
