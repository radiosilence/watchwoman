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
                // `CARGO_BIN_EXE_watchwoman` is only set for integration
                // tests inside the same package as the binary; our tests
                // live in a sibling crate, so we fall back to locating
                // the binary in the workspace target dir.
                if let Some(p) = std::env::var_os("CARGO_BIN_EXE_watchwoman") {
                    return Ok(PathBuf::from(p));
                }
                locate_in_target("watchwoman")
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

fn locate_in_target(bin: &str) -> anyhow::Result<PathBuf> {
    // Walk up from this crate's manifest dir looking for `target/<profile>/<bin>`.
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").context("no CARGO_MANIFEST_DIR")?;
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let mut cur: &Path = Path::new(&manifest_dir);
    loop {
        let candidate = cur.join("target").join(profile).join(bin);
        if candidate.is_file() {
            return Ok(candidate);
        }
        let Some(parent) = cur.parent() else { break };
        cur = parent;
    }
    bail!("{bin} not found in any target/{profile} directory above {manifest_dir}")
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

    /// Spawn with extra env vars set on the daemon process — used by
    /// tests that need to flip a runtime knob like
    /// `WATCHWOMAN_STALE_IDLE_SECS` without polluting the test
    /// process's own environment (cargo runs tests in shared
    /// process-wide env, so global mutation is racy).
    pub fn spawn_with_env(envs: &[(&str, &str)]) -> anyhow::Result<Self> {
        Self::spawn_inner(TargetBinary::from_env(), envs)
    }

    pub fn spawn_with(target: TargetBinary) -> anyhow::Result<Self> {
        Self::spawn_inner(target, &[])
    }

    fn spawn_inner(target: TargetBinary, envs: &[(&str, &str)]) -> anyhow::Result<Self> {
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

        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("WATCHMAN_CONFIG_FILE", "/dev/null");
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let child = cmd
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
