//! Daemon runtime: unix socket listener, per-root state, notify-backed
//! watcher. One process serves every root.

use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::Context;

pub mod clock;
pub mod root;
pub mod scm;
pub mod server;
pub mod session;
pub mod state;
pub mod tree;
pub mod watcher;

pub use session::Session;
pub use state::DaemonState;

/// Run the daemon in the foreground.  Used by `--foreground-daemon` and
/// by the integration harness.
pub fn run_foreground(sock: &Path) -> anyhow::Result<ExitCode> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    rt.block_on(async move {
        let state = Arc::new(DaemonState::new(sock.to_path_buf()));
        server::serve(state).await?;
        Ok::<_, anyhow::Error>(())
    })?;
    Ok(ExitCode::SUCCESS)
}
