use anyhow::Result;

/// Runs a named suite of `.slt` files under `tests/<suite>/` against a
/// running zeta-mysqlwire endpoint via sqllogictest.
pub async fn run_suite(_suite: &str, _filter: Option<&str>) -> Result<()> {
    // TODO: glob tests/<suite>/**/*.slt, instantiate sqllogictest::Runner
    // with a mysql_async-based DB connector, execute, honor skip_list.toml.
    anyhow::bail!("slt_runner::run_suite: not yet implemented")
}
