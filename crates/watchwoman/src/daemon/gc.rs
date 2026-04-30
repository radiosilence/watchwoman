//! Watch garbage collector.
//!
//! Zero-conf background task that periodically reaps watches that
//! are:
//!
//!   - **dead** — the root directory is missing from disk on two
//!     consecutive ticks (~60-120 s grace), usually because a git
//!     worktree got removed, a volume unmounted, or a scratch dir was
//!     nuked.  Reaping frees the fsevents/inotify registration plus
//!     all the tree state immediately.
//!
//!   - **stale** — no subscriptions, no triggers, and no commands have
//!     touched the root for [`STALE_IDLE_SECS`] seconds.  Long-running
//!     daemons (the macOS LaunchAgent in particular) otherwise keep
//!     every one-shot `watch-project` forever.
//!
//! Active roots — anything with a subscription or installed trigger —
//! are never stale-reaped, regardless of idle time.  They can still
//! be dead-reaped: if the directory is gone, nothing useful is
//! happening anyway.
//!
//! Every sweep also prunes tombstones (deleted-file entries) from
//! each surviving root.  Subscribers ship deletions live via tick
//! events, so we only need to keep tombstones around for named
//! cursors that haven't caught up yet — the watermark is the oldest
//! live cursor.  Keeps the tree from growing unboundedly as files
//! churn through branch switches and build output.

use std::sync::Arc;
use std::time::Duration;

use tokio::time;

use super::alloc;
use super::state::{DaemonState, ReapEvent, ReapReason};

/// How often the GC loop wakes up.
const TICK: Duration = Duration::from_secs(60);

/// Consecutive missing-from-disk ticks before a root is reaped as dead.
/// Two ticks means ~60-120 s grace — enough to ride out the odd
/// rename-in-place or brief unmount without prematurely reaping.
const DEAD_TICK_THRESHOLD: u32 = 2;

/// Default idle seconds that a root with no subscriptions and no
/// triggers is tolerated before it's reaped as stale.  Watches are
/// ephemeral — when a tool like `watchman-wait`, `jest --watch` or a
/// metro server exits, the root sits unused with no client to clean
/// up after it.  An hour is long enough to ride out a coffee break or
/// a flaky CI rerun, short enough that abandoned worktrees don't
/// accumulate file index data for days on end.
///
/// Override at daemon start with `WATCHWOMAN_STALE_IDLE_SECS=<n>` —
/// `0` disables stale reaping entirely (dead reaping still runs).
const STALE_IDLE_SECS_DEFAULT: u64 = 60 * 60;

/// Resolve the active stale-reap threshold from the environment, with
/// the default as a fallback.  Read once per sweep so the user can
/// twiddle it without bouncing the daemon (cheap; envs are a syscall).
/// Pub for `info::status` so the health classifier mirrors what the
/// GC will actually reap.
pub fn stale_idle_secs() -> u64 {
    std::env::var("WATCHWOMAN_STALE_IDLE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(STALE_IDLE_SECS_DEFAULT)
}

/// Spawn the GC task.  It lives for the lifetime of the daemon and
/// exits on shutdown.
pub fn spawn(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        let mut ticker = time::interval(TICK);
        // The first tick fires immediately with tokio's default —
        // skip it so we don't reap within milliseconds of startup,
        // before clients have had a chance to register watches.
        ticker.tick().await;
        loop {
            tokio::select! {
                _ = ticker.tick() => sweep(&state),
                _ = state.shutdown.notified() => break,
            }
            if state.is_shutting_down() {
                break;
            }
        }
    });
}

/// Run one reap sweep synchronously.  Exposed for the hidden
/// `debug-gc-tick` command so integration tests can exercise the
/// policy without waiting a real minute.
pub fn sweep(state: &DaemonState) {
    let stale_secs = stale_idle_secs();
    let paths: Vec<_> = state.roots.iter().map(|e| e.key().clone()).collect();
    for path in paths {
        let Some(root) = state.roots.get(&path).map(|r| r.clone()) else {
            continue;
        };

        // Dead check — stat() is the only cross-filesystem-safe signal
        // (a symlink-target check would misfire on network mounts).
        let exists = std::fs::metadata(&path)
            .map(|m| m.is_dir())
            .unwrap_or(false);
        if !exists {
            let misses = root.mark_missing();
            if misses >= DEAD_TICK_THRESHOLD {
                reap(state, &path, ReapReason::Dead);
                continue;
            }
            // Not yet — come back next tick.  Skip the stale check:
            // a missing-but-not-yet-dead root can't be meaningfully
            // "idle", and we don't want to double-reap.
            continue;
        }
        root.mark_present();

        // Stale check — only applies to roots with no active clients.
        // A subscribed or triggered root is in use by definition, no
        // matter how long since the last explicit command.  A 0
        // threshold disables this branch entirely.
        if stale_secs > 0
            && root.subscription_count() == 0
            && root.trigger_count() == 0
            && root.idle_seconds() >= stale_secs
        {
            reap(state, &path, ReapReason::Stale);
            continue;
        }

        // Tombstone pruning — the big memory saver for roots that see
        // a lot of file churn (branch switches, build output).  The
        // tree-side guard makes this a cheap no-op when there's
        // nothing to prune.
        let (watermark, removed) = root.prune_tombstones();
        if removed > 0 {
            tracing::debug!(
                path = %path.display(),
                watermark,
                removed,
                "pruned tombstones"
            );
        }
    }
}

fn reap(state: &DaemonState, path: &std::path::Path, reason: ReapReason) {
    if !state.unregister_root(path) {
        return;
    }
    tracing::warn!(
        path = %path.display(),
        reason = reason.as_str(),
        "garbage-collected watch"
    );
    state.log_reap(ReapEvent {
        path: path.to_path_buf(),
        reason,
        at_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    });
    // Reaping a root with millions of file entries can free hundreds
    // of MB at once.  Without this nudge the allocator just parks the
    // pages on its dirty list, so RSS lies — exactly the regression
    // that prompted this whole change.
    alloc::purge();
}
