use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, ExitCode, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::{CommandFactory, Parser};
use clap_complete::{generate, Shell};
use indexmap::IndexMap;
use watchwoman_protocol::{json, Value};

use crate::sock;

#[derive(Debug, Parser)]
#[command(
    name = "watchwoman",
    version,
    about = "A drop-in watchman replacement that doesn't eat your RAM.",
    long_about = "watchwoman speaks the watchman wire protocol and CLI. Drop a \
                  `watchman` symlink next to the binary and every tool that \
                  expects watchman will talk to us instead."
)]
pub struct Cli {
    /// Path to the unix socket.  Falls back to $WATCHMAN_SOCK, then a
    /// platform default under $XDG_STATE_HOME (zeroconf).
    #[arg(long, env = "WATCHMAN_SOCK", global = true)]
    pub sockname: Option<String>,

    /// Select wire encoding for socket output. Defaults to JSON for CLI use.
    #[arg(long, global = true, default_value = "json")]
    pub output_encoding: Encoding,

    /// Select wire encoding expected from the server. Defaults to JSON.
    #[arg(long, global = true, default_value = "json")]
    pub server_encoding: Encoding,

    /// Compact JSON output instead of pretty-printed.
    #[arg(long, global = true)]
    pub no_pretty: bool,

    /// Don't auto-spawn the daemon if the socket is missing.
    #[arg(long, global = true)]
    pub no_spawn: bool,

    /// Read a JSON PDU from stdin and send it to the daemon directly.
    /// Used by `git fsmonitor`, Sapling, Metro, and every other tool
    /// that speaks watchman's PDU protocol without the subcommand CLI.
    /// Pairs with `--persistent` for subscribe streams.
    #[arg(short = 'j', long = "json-command", global = true)]
    pub json_command: bool,

    /// Stay connected after the first response and stream unilateral
    /// PDUs (subscription updates, state-broadcasts) until EOF or
    /// the daemon closes the connection.  Matches watchman's `-p`.
    #[arg(short = 'p', long = "persistent", global = true)]
    pub persistent: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
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
    /// Print shell completion script for the given shell.
    Completion {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Print the path to the unix socket.
    GetSockname,
    /// Print the daemon's PID.
    GetPid,
    /// Print the watchman-compatible version and capability probe result.
    Version {
        /// Required capabilities (comma-separated).
        #[arg(long, value_delimiter = ',')]
        required: Vec<String>,
        /// Optional capabilities (comma-separated).
        #[arg(long, value_delimiter = ',')]
        optional: Vec<String>,
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
    /// Run a structured query.  Pass the query spec as a JSON blob.
    Query { path: String, query: String },
    /// Subscribe to a root; prints an initial response and exits.
    Subscribe {
        path: String,
        name: String,
        query: String,
    },
    /// Unsubscribe from a named subscription.
    Unsubscribe { path: String, name: String },
    /// Wait for pending subscriptions to flush.
    FlushSubscriptions {
        path: String,
        #[arg(long, default_value_t = 5000)]
        timeout_ms: u64,
    },
    /// Enter a named state on a root.
    StateEnter { path: String, name: String },
    /// Leave a named state on a root.
    StateLeave { path: String, name: String },
    /// Fetch the watchmanconfig for a root.
    GetConfig { path: String },
    /// Set or read the server log level.
    LogLevel { level: Option<String> },
    /// Write a message to the server log.
    Log { level: String, message: String },
    /// Tear the daemon down.
    ShutdownServer,
    /// Send an arbitrary JSON PDU and print the response.
    Raw {
        /// `["command", "arg1", {"key":"val"}]` shape.
        pdu: String,
    },
}

pub fn run() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();
    init_tracing();

    let sock_path = sock::resolve(cli.sockname.as_deref())?;
    tracing::debug!(?sock_path, "resolved socket path");

    // `-j` / `--json-command` reads the PDU from stdin and skips
    // subcommand parsing. Mutually exclusive with a subcommand.
    if cli.json_command {
        return run_stdin_json(&sock_path, cli.no_pretty, cli.no_spawn, cli.persistent);
    }

    let Some(cmd) = cli.command else {
        // `watchman` with no args prints help and exits 1, matching
        // the upstream behaviour.
        Cli::command().print_help()?;
        println!();
        return Ok(ExitCode::from(1));
    };

    match cmd {
        Command::ForegroundDaemon => crate::daemon::run_foreground(&sock_path),
        Command::Completion { shell } => {
            let mut command = Cli::command();
            generate(shell, &mut command, "watchwoman", &mut io::stdout());
            Ok(ExitCode::SUCCESS)
        }
        other => run_client(
            &other,
            &sock_path,
            cli.no_pretty,
            cli.no_spawn,
            cli.persistent,
        ),
    }
}

fn init_tracing() {
    // `RUST_LOG=debug watchwoman ...` enables verbose tracing; silent by default.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn run_client(
    cmd: &Command,
    sock_path: &Path,
    no_pretty: bool,
    no_spawn: bool,
    persistent: bool,
) -> anyhow::Result<ExitCode> {
    let pdu = build_pdu(cmd)?;
    send_and_print(&pdu, sock_path, no_pretty, no_spawn, persistent)
}

fn run_stdin_json(
    sock_path: &Path,
    no_pretty: bool,
    no_spawn: bool,
    persistent: bool,
) -> anyhow::Result<ExitCode> {
    let mut buf = String::new();
    io::stdin()
        .read_to_string(&mut buf)
        .context("reading PDU from stdin")?;
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        anyhow::bail!("`-j` expected a JSON PDU on stdin");
    }
    let json: serde_json::Value = serde_json::from_str(trimmed).context("parsing stdin PDU")?;
    let pdu = json_to_value(json);
    send_and_print(&pdu, sock_path, no_pretty, no_spawn, persistent)
}

fn send_and_print(
    pdu: &Value,
    sock_path: &Path,
    no_pretty: bool,
    no_spawn: bool,
    persistent: bool,
) -> anyhow::Result<ExitCode> {
    if !sock_path.exists() && !no_spawn {
        spawn_daemon(sock_path)?;
    }

    let stream = std::os::unix::net::UnixStream::connect(sock_path).with_context(|| {
        format!(
            "connecting to {} — is the daemon running? (try `watchwoman --foreground-daemon`)",
            sock_path.display()
        )
    })?;
    // No read timeout in persistent mode — we want to block waiting for
    // unilateral PDUs.  One-shot mode caps at 30s so a wedged daemon
    // doesn't hang a shell.
    if !persistent {
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    }
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;

    let mut writer = stream.try_clone()?;
    json::encode_pdu(&mut writer, pdu)?;
    writer.flush()?;

    let mut reader = std::io::BufReader::new(stream);
    let response = json::read_pdu(&mut reader)?.context("daemon closed connection early")?;

    print_response(&response, no_pretty)?;
    let err_present = response
        .as_object()
        .is_some_and(|o| o.contains_key("error"));

    if persistent {
        // Drain subsequent unilateral PDUs until the daemon closes the
        // connection or the user hits SIGINT.
        while let Some(v) = json::read_pdu(&mut reader)? {
            print_response(&v, no_pretty)?;
        }
    }

    Ok(if err_present {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn build_pdu(cmd: &Command) -> anyhow::Result<Value> {
    let mut parts: Vec<Value> = Vec::with_capacity(4);
    match cmd {
        Command::ForegroundDaemon | Command::Completion { .. } => {
            unreachable!("handled by caller")
        }
        Command::GetSockname => parts.push(Value::String("get-sockname".into())),
        Command::GetPid => parts.push(Value::String("get-pid".into())),
        Command::Version { required, optional } => {
            parts.push(Value::String("version".into()));
            if !required.is_empty() || !optional.is_empty() {
                let mut m = IndexMap::new();
                if !required.is_empty() {
                    m.insert(
                        "required".into(),
                        Value::Array(
                            required
                                .iter()
                                .cloned()
                                .map(Value::String)
                                .collect::<Vec<_>>(),
                        ),
                    );
                }
                if !optional.is_empty() {
                    m.insert(
                        "optional".into(),
                        Value::Array(
                            optional
                                .iter()
                                .cloned()
                                .map(Value::String)
                                .collect::<Vec<_>>(),
                        ),
                    );
                }
                parts.push(Value::Object(m));
            }
        }
        Command::ListCapabilities => parts.push(Value::String("list-capabilities".into())),
        Command::WatchProject { path } => {
            parts.push(Value::String("watch-project".into()));
            parts.push(Value::String(absolutise(path)));
        }
        Command::Watch { path } => {
            parts.push(Value::String("watch".into()));
            parts.push(Value::String(absolutise(path)));
        }
        Command::WatchList => parts.push(Value::String("watch-list".into())),
        Command::WatchDel { path } => {
            parts.push(Value::String("watch-del".into()));
            parts.push(Value::String(absolutise(path)));
        }
        Command::WatchDelAll => parts.push(Value::String("watch-del-all".into())),
        Command::Clock { path } => {
            parts.push(Value::String("clock".into()));
            parts.push(Value::String(absolutise(path)));
        }
        Command::Query { path, query } => {
            parts.push(Value::String("query".into()));
            parts.push(Value::String(absolutise(path)));
            parts.push(parse_json(query, "query")?);
        }
        Command::Subscribe { path, name, query } => {
            parts.push(Value::String("subscribe".into()));
            parts.push(Value::String(absolutise(path)));
            parts.push(Value::String(name.clone()));
            parts.push(parse_json(query, "subscribe")?);
        }
        Command::Unsubscribe { path, name } => {
            parts.push(Value::String("unsubscribe".into()));
            parts.push(Value::String(absolutise(path)));
            parts.push(Value::String(name.clone()));
        }
        Command::FlushSubscriptions { path, timeout_ms } => {
            parts.push(Value::String("flush-subscriptions".into()));
            parts.push(Value::String(absolutise(path)));
            parts.push(Value::Int(*timeout_ms as i64));
        }
        Command::StateEnter { path, name } => {
            parts.push(Value::String("state-enter".into()));
            parts.push(Value::String(absolutise(path)));
            parts.push(Value::String(name.clone()));
        }
        Command::StateLeave { path, name } => {
            parts.push(Value::String("state-leave".into()));
            parts.push(Value::String(absolutise(path)));
            parts.push(Value::String(name.clone()));
        }
        Command::GetConfig { path } => {
            parts.push(Value::String("get-config".into()));
            parts.push(Value::String(absolutise(path)));
        }
        Command::LogLevel { level } => {
            parts.push(Value::String("log-level".into()));
            if let Some(l) = level {
                parts.push(Value::String(l.clone()));
            }
        }
        Command::Log { level, message } => {
            parts.push(Value::String("log".into()));
            parts.push(Value::String(level.clone()));
            parts.push(Value::String(message.clone()));
        }
        Command::ShutdownServer => parts.push(Value::String("shutdown-server".into())),
        Command::Raw { pdu } => {
            return parse_json(pdu, "raw PDU");
        }
    }
    Ok(Value::Array(parts))
}

fn parse_json(s: &str, ctx: &str) -> anyhow::Result<Value> {
    let j: serde_json::Value =
        serde_json::from_str(s).with_context(|| format!("parsing {ctx} JSON"))?;
    Ok(json_to_value(j))
}

fn json_to_value(v: serde_json::Value) -> Value {
    use serde_json::Value as J;
    match v {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(b),
        J::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Real(f)
            } else {
                Value::Null
            }
        }
        J::String(s) => Value::String(s),
        J::Array(a) => Value::Array(a.into_iter().map(json_to_value).collect()),
        J::Object(o) => {
            let mut m = IndexMap::with_capacity(o.len());
            for (k, val) in o {
                m.insert(k, json_to_value(val));
            }
            Value::Object(m)
        }
    }
}

fn absolutise(path: &str) -> String {
    match std::fs::canonicalize(path) {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(_) => {
            if Path::new(path).is_absolute() {
                path.to_owned()
            } else {
                let cwd = std::env::current_dir().unwrap_or_default();
                cwd.join(path).to_string_lossy().into_owned()
            }
        }
    }
}

fn print_response(v: &Value, no_pretty: bool) -> anyhow::Result<()> {
    let j = value_to_serde(v);
    let mut out = io::stdout().lock();
    if no_pretty {
        serde_json::to_writer(&mut out, &j)?;
    } else {
        serde_json::to_writer_pretty(&mut out, &j)?;
    }
    out.write_all(b"\n")?;
    Ok(())
}

fn value_to_serde(v: &Value) -> serde_json::Value {
    use serde_json::{Number, Value as J};
    match v {
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Int(i) => J::Number(Number::from(*i)),
        Value::Real(f) => Number::from_f64(*f).map(J::Number).unwrap_or(J::Null),
        Value::String(s) => J::String(s.clone()),
        Value::Bytes(b) => J::String(String::from_utf8_lossy(b).into_owned()),
        Value::Array(a) => J::Array(a.iter().map(value_to_serde).collect()),
        Value::Object(o) => {
            let mut map = serde_json::Map::new();
            for (k, val) in o {
                map.insert(k.clone(), value_to_serde(val));
            }
            J::Object(map)
        }
        Value::Template { keys, rows } => {
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                let mut obj = serde_json::Map::new();
                for (k, val) in keys.iter().zip(row.iter()) {
                    obj.insert(k.clone(), value_to_serde(val));
                }
                out.push(J::Object(obj));
            }
            J::Array(out)
        }
    }
}

fn spawn_daemon(sock_path: &Path) -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("resolving current_exe")?;
    if let Some(parent) = sock_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let log_path = sock_path.with_extension("log");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .context("opening daemon log")?;
    let log_err = log.try_clone()?;

    let mut cmd = StdCommand::new(&exe);
    cmd.arg("--sockname")
        .arg(sock_path)
        .arg("--foreground-daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));

    // Detach from the controlling terminal so the daemon keeps running
    // once the CLI exits.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        unsafe {
            cmd.pre_exec(|| {
                // setsid() — detach from parent session so signals don't
                // cascade.  Safe because we run in a freshly-forked child.
                if libc_setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    let _child = cmd.spawn().context("spawning daemon")?;

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if sock_path.exists() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    anyhow::bail!(
        "daemon spawned but socket {} never appeared; see {} for details",
        sock_path.display(),
        log_path.display()
    )
}

#[cfg(unix)]
fn libc_setsid() -> i32 {
    // SAFETY: setsid has no Rust prerequisites — it just creates a new
    // session on the calling process.  We are in a child about to exec.
    unsafe { libc_ffi::setsid() }
}

#[cfg(unix)]
mod libc_ffi {
    extern "C" {
        pub fn setsid() -> i32;
    }
}

fn _clap_factory_retained() {
    // Force clap::CommandFactory to be considered used — some toolchains
    // warn on import-only derives when the generated code changes.
    let _ = Cli::command;
}

#[allow(dead_code)]
fn _ensure_pathbuf_in_scope(_p: PathBuf) {}
