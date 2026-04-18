use std::path::Path;

use anyhow::Context;

/// Run the daemon in the foreground.  Intended for `--foreground-daemon`
/// and for tests that want a bound socket without forking.
pub fn run_foreground(_sock: &Path) -> anyhow::Result<std::process::ExitCode> {
    let _rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    // TODO: bind unix socket, accept connections, dispatch commands.
    anyhow::bail!("daemon loop not yet implemented")
}
