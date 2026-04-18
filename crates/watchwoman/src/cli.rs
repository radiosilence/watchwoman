use std::process::ExitCode;

use clap::Parser;

use crate::sock;

#[derive(Debug, Parser)]
#[command(
    name = "watchwoman",
    version,
    about = "A drop-in watchman replacement."
)]
pub struct Cli {
    /// Path to the unix socket. Falls back to $WATCHMAN_SOCK, then a
    /// platform default under $XDG_STATE_HOME.
    #[arg(long, env = "WATCHMAN_SOCK", global = true)]
    pub sockname: Option<String>,

    /// Select wire encoding for socket output. Defaults to JSON for CLI use.
    #[arg(long, global = true, default_value = "json")]
    pub output_encoding: Encoding,

    /// Select wire encoding expected from the server. Defaults to JSON.
    #[arg(long, global = true, default_value = "json")]
    pub server_encoding: Encoding,

    /// Silence the informational header in JSON output.
    #[arg(long, global = true)]
    pub no_pretty: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum Encoding {
    Json,
    Bser,
    Bser2,
}

#[derive(Debug, clap::Subcommand)]
pub enum Command {
    /// Run the watchwoman daemon in the foreground.
    #[command(name = "--foreground-daemon", hide = true)]
    ForegroundDaemon,
    /// Print the path to the unix socket.
    GetSockname,
    /// Print the daemon's PID.
    GetPid,
    /// Print the watchman-compatible version and capability probe result.
    Version {
        /// Optional JSON object with `required` / `optional` capability arrays.
        #[arg(trailing_var_arg = true)]
        capabilities: Vec<String>,
    },
    /// List every capability the daemon advertises.
    ListCapabilities,
    /// Watch a path and return the enclosing project root.
    WatchProject { path: String },
    /// Watch a raw path without project-root resolution.
    Watch { path: String },
    /// Enumerate every currently watched root.
    WatchList,
    /// Stop watching a root.
    WatchDel { path: String },
    /// Stop watching every root.
    WatchDelAll,
    /// Return the clock value for a root.
    Clock { path: String },
    /// Run a structured query against a root.
    Query {
        path: String,
        #[arg(trailing_var_arg = true)]
        query: Vec<String>,
    },
    /// Tear the daemon down.
    ShutdownServer,
}

pub fn run() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();
    let sock_path = sock::resolve(cli.sockname.as_deref())?;
    tracing::debug!(?sock_path, "resolved socket path");

    match &cli.command {
        Command::ForegroundDaemon => crate::daemon::run_foreground(&sock_path),
        other => {
            // Client-side path wires up in a later ticket; fail loudly
            // rather than silently to keep parity regressions visible.
            anyhow::bail!("client command {other:?} not wired yet (MVP in progress)")
        }
    }
}
