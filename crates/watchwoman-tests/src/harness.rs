use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use tempfile::TempDir;

use crate::Client;

/// Which binary the harness should exercise.
#[derive(Debug, Clone)]
pub enum TargetBinary {
    /// Our binary, located via `CARGO_BIN_EXE_watchwoman`.
    Watchwoman,
    /// Real watchman on `$PATH` — used to record ground-truth fixtures.
    Watchman,
    /// Arbitrary binary — used by the fixture recorder.
    Explicit(PathBuf),
}

impl TargetBinary {
    pub fn from_env() -> Self {
        match std::env::var("WATCHWOMAN_UNDER_TEST").ok().as_deref() {
            Some("watchman") => TargetBinary::Watchman,
            Some(other) if !other.is_empty() => TargetBinary::Explicit(PathBuf::from(other)),
            _ => TargetBinary::Watchwoman,
        }
    }

    pub fn resolve(&self) -> anyhow::Result<PathBuf> {
        match self {
            TargetBinary::Watchwoman => {
                // Cargo sets this env var for tests so we don't need to
                // shell out to `which` and risk hitting a stale install.
                let p = std::env::var_os("CARGO_BIN_EXE_watchwoman")
                    .context("CARGO_BIN_EXE_watchwoman not set; run under `cargo test`")?;
                Ok(PathBuf::from(p))
            }
            TargetBinary::Watchman => which("watchman"),
            TargetBinary::Explicit(p) => Ok(p.clone()),
        }
    }
}

fn which(bin: &str) -> anyhow::Result<PathBuf> {
    let path = std::env::var_os("PATH").context("no $PATH")?;
    for entry in std::env::split_paths(&path) {
        let candidate = entry.join(bin);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!("{bin} not found on $PATH")
}

/// One daemon + one state dir, torn down when dropped.
pub struct Harness {
    _state_dir: TempDir,
    sock_path: PathBuf,
    child: Option<Child>,
    target: TargetBinary,
}

impl Harness {
    /// Spawn the target binary with an isolated socket + state dir.
    pub fn spawn() -> anyhow::Result<Self> {
        Self::spawn_with(TargetBinary::from_env())
    }

    pub fn spawn_with(target: TargetBinary) -> anyhow::Result<Self> {
        let state_dir = tempfile::Builder::new()
            .prefix("watchwoman-harness-")
            .tempdir()
            .context("creating scratch state dir")?;
        let sock_path = state_dir.path().join("sock");

        let binary = target.resolve()?;
        let mut cmd = Command::new(&binary);
        match &target {
            TargetBinary::Watchwoman | TargetBinary::Explicit(_) => {
                cmd.arg("--sockname")
                    .arg(&sock_path)
                    .arg("--foreground-daemon");
            }
            TargetBinary::Watchman => {
                cmd.arg("--foreground")
                    .arg("--sockname")
                    .arg(&sock_path)
                    .arg("--logfile")
                    .arg(state_dir.path().join("log"))
                    .arg("--statefile")
                    .arg(state_dir.path().join("state"))
                    .arg("--pidfile")
                    .arg(state_dir.path().join("pid"));
            }
        }

        let child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("WATCHMAN_CONFIG_FILE", "/dev/null")
            .spawn()
            .with_context(|| format!("spawning {}", binary.display()))?;

        let harness = Harness {
            _state_dir: state_dir,
            sock_path,
            child: Some(child),
            target,
        };
        harness.wait_for_socket(Duration::from_secs(5))?;
        Ok(harness)
    }

    pub fn sock(&self) -> &Path {
        &self.sock_path
    }

    pub fn target(&self) -> &TargetBinary {
        &self.target
    }

    /// Open a fresh JSON client connection.
    pub fn client(&self) -> anyhow::Result<Client> {
        Client::connect(&self.sock_path)
    }

    fn wait_for_socket(&self, timeout: Duration) -> anyhow::Result<()> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.sock_path.exists() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        bail!(
            "daemon did not bind {} within {:?}",
            self.sock_path.display(),
            timeout
        )
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // Politely first, then forcibly.
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
