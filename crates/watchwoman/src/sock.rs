use std::path::{Path, PathBuf};

use anyhow::Context;

/// Watchman picks a socket path in this precedence:
///
/// 1. `--sockname` / `$WATCHMAN_SOCK`
/// 2. `$XDG_STATE_HOME/watchman/<user>-state/sock` (Linux)
/// 3. `~/.local/state/watchman/<user>-state/sock` (macOS, matches brew)
/// 4. `$TMPDIR/<user>-state/sock`
///
/// The brew-packaged watchman on this machine resolves to #3, so we
/// match that first.
pub fn resolve(explicit: Option<&str>) -> anyhow::Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(PathBuf::from(p));
    }

    let user = whoami().context("resolving current user")?;

    if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
        return Ok(Path::new(&state)
            .join("watchman")
            .join(format!("{user}-state"))
            .join("sock"));
    }

    if let Some(home) = std::env::var_os("HOME") {
        return Ok(Path::new(&home)
            .join(".local/state/watchman")
            .join(format!("{user}-state"))
            .join("sock"));
    }

    let tmp = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
    Ok(Path::new(&tmp).join(format!("{user}-state")).join("sock"))
}

fn whoami() -> anyhow::Result<String> {
    // Prefer $USER so tests can override it cleanly without setuid weirdness.
    if let Ok(u) = std::env::var("USER") {
        if !u.is_empty() {
            return Ok(u);
        }
    }
    let uid = nix::unistd::geteuid();
    let user = nix::unistd::User::from_uid(uid)
        .ok()
        .flatten()
        .context("no passwd entry for current uid")?;
    Ok(user.name)
}
