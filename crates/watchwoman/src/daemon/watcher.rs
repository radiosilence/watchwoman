//! notify-backed filesystem watcher.  One watcher per root; coalesces
//! events into tick batches before the tree sees them.
//!
//! We intentionally do not stream every event individually — that's
//! watchman's most common source of "too many inotify watches" and
//! "recrawl" noise.  Batching at 5 ms matches watchman's default
//! settle period well enough for CLI tools.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use super::jj_scope::JjScope;
use super::root::{PathChange, Root, WatchIgnores, WatcherCommand};

const SETTLE: Duration = Duration::from_millis(5);
const MAX_BATCH: usize = 1024;

/// Start the watch for `root`.
///
/// Takes `&Arc` rather than an owned one on purpose: nothing spawned
/// here may hold a strong reference.  `Root`'s `Drop` is what tells
/// these tasks to stop, so a strong `Arc` in the event loop would keep
/// the root — and its entire file tree arena — alive for the life of
/// the daemon, waiting for a shutdown that can't be sent until it lets
/// go.  Every root watched would leak its full index.
pub fn spawn(
    root: &Arc<Root>,
    mut cmd_rx: mpsc::UnboundedReceiver<WatcherCommand>,
) -> anyhow::Result<()> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<notify::Result<Event>>();

    let root_path = root.path.clone();
    // Synchronous rendezvous so `register_root` can't return — and
    // `watch-project` can't ack the client — until the kernel-side
    // inotify / fsevents registration is actually live.  Without this
    // a fast `write()` immediately after `watch-project` would land
    // before the watch was attached and the event would be silently
    // dropped.  Saw it as a 10–15 % flake on the trigger integration
    // test before this barrier went in.
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(0);
    // Shutdown path for the blocking thread below.  `JoinHandle::abort`
    // is useless here: a `spawn_blocking` task that has already started
    // never observes cancellation, so aborting left the thread parked
    // forever holding a live fsevents/inotify registration — one leaked
    // thread and one leaked kernel watch per root.
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    tokio::task::spawn_blocking(move || {
        let mut watcher: RecommendedWatcher = match notify::recommended_watcher(move |res| {
            let _ = event_tx.send(res);
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(?e, "failed to create watcher");
                let _ = ready_tx.send(Err(e.to_string()));
                return;
            }
        };
        if let Err(e) = watcher.watch(&root_path, RecursiveMode::Recursive) {
            tracing::warn!(?e, "failed to watch root");
            let _ = ready_tx.send(Err(e.to_string()));
            return;
        }
        // Signal ready *after* `watch()` returns: at this point
        // inotify/fsevents has the kernel registration in place, so
        // anything that touches the tree from now on will produce an
        // event.  Closing the sender on early-failure paths above
        // unblocks the caller too — they'll proceed without a working
        // watcher, the warn above being the only signal, which
        // matches the previous behaviour.
        let _ = ready_tx.send(Ok(()));
        // Hold the watcher alive until the event loop is done with it.
        // Returning from here drops `watcher`, which is what actually
        // unregisters the kernel-side watch.
        let _ = stop_rx.recv();
    });
    // Block until the watcher is registered.  The blocking task above
    // runs on tokio's blocking-thread pool, so this doesn't deadlock
    // the runtime even when called from an async handler.
    ready_rx.recv()?.map_err(anyhow::Error::msg)?;

    let root_for_events = Arc::downgrade(root);
    tokio::spawn(async move {
        let mut jj_scope = JjScope::default();
        loop {
            tokio::select! {
                biased;
                cmd = cmd_rx.recv() => match cmd {
                    Some(WatcherCommand::Shutdown) | None => break,
                },
                first = event_rx.recv() => {
                    let Some(first) = first else { break };
                    // Unregistered between the event firing and us
                    // waking up — nothing left to apply it to.
                    let Some(root) = root_for_events.upgrade() else { break };
                    let mut events = vec![first];
                    let deadline = tokio::time::Instant::now() + SETTLE;
                    while events.len() < MAX_BATCH {
                        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                        if remaining.is_zero() { break; }
                        match tokio::time::timeout(remaining, event_rx.recv()).await {
                            Ok(Some(next)) => events.push(next),
                            Ok(None) | Err(_) => break,
                        }
                    }
                    let _mode_guard = root.mode_transition.read();
                    let jj_mode = root.is_jj_events();
                    let mut batch = Vec::with_capacity(events.len());
                    for event in events {
                        collect_event(&root, event, &mut batch, &mut jj_scope, jj_mode);
                    }
                    if !batch.is_empty() {
                        root.apply_changes(batch);
                    }
                }
            }
        }
        // Releases the blocking thread, dropping the notify watcher
        // with it.
        drop(stop_tx);
    });
    Ok(())
}

fn collect_event(
    root: &Root,
    event: notify::Result<Event>,
    out: &mut Vec<PathChange>,
    jj_scope: &mut JjScope,
    jj_mode: bool,
) {
    let event = match event {
        Ok(ev) => ev,
        Err(e) => {
            tracing::debug!(?e, "notify error");
            if root.invalidate_jj_events() {
                jj_scope.clear();
                out.clear();
            }
            return;
        }
    };
    if event.need_rescan() {
        if root.invalidate_jj_events() {
            jj_scope.clear();
            out.clear();
        }
        return;
    }
    if matches!(event.kind, EventKind::Access(_)) {
        return;
    }
    for abs in &event.paths {
        let Some(rel) = abs.strip_prefix(&root.path).ok().map(PathBuf::from) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        if (jj_mode && is_jj_internal(&rel)) || (!jj_mode && should_ignore(&rel, &root.ignores)) {
            continue;
        }
        if jj_mode && rel.file_name().is_some_and(|name| name == ".gitignore") {
            jj_scope.clear();
            root.invalidate_jj_events();
            out.clear();
            continue;
        }
        if jj_mode {
            let is_dir = std::fs::symlink_metadata(abs).is_ok_and(|metadata| metadata.is_dir())
                || matches!(event.kind, EventKind::Remove(_));
            if let Some(ignored) = jj_scope.ignored_directory(&root.path, &rel, is_dir) {
                if ignored == rel && matches!(event.kind, EventKind::Remove(_)) {
                    out.push(PathChange::Remove { rel: rel.clone() });
                }
                out.push(PathChange::Signal {
                    rel: ignored.join(".gitignore"),
                });
                continue;
            }
        }
        match event.kind {
            EventKind::Remove(_) => {
                out.push(PathChange::Remove { rel });
            }
            _ => {
                let Ok(metadata) = std::fs::symlink_metadata(abs) else {
                    // File disappeared before we could stat — treat as removal.
                    out.push(PathChange::Remove { rel });
                    continue;
                };
                let symlink_target = if metadata.is_symlink() {
                    std::fs::read_link(abs)
                        .ok()
                        .map(|p| p.to_string_lossy().into_owned())
                } else {
                    None
                };
                out.push(PathChange::Upsert {
                    rel,
                    metadata,
                    symlink_target,
                });
            }
        }
    }
}

fn is_jj_internal(rel: &Path) -> bool {
    rel.components()
        .next()
        .is_some_and(|component| component.as_os_str() == ".git" || component.as_os_str() == ".jj")
}

pub(crate) fn initial_scan(
    root: &Path,
    ignores: &WatchIgnores,
) -> Vec<(PathBuf, std::fs::Metadata, Option<String>)> {
    // Manual recursive walk so we can prune at the directory level —
    // descending into `node_modules` before filtering was dominating
    // the initial scan on any large JS project.
    let mut out = Vec::new();
    walk(root, Path::new(""), ignores, &mut out, false);
    out
}

/// `shallow` is true once we've descended into a configured VCS directory.
/// Its immediate contents are reported, but deeper entries are skipped.
/// `ignore_dirs` prunes the entire subtree.
fn walk(
    abs_dir: &std::path::Path,
    rel_dir: &std::path::Path,
    ignores: &WatchIgnores,
    out: &mut Vec<(PathBuf, std::fs::Metadata, Option<String>)>,
    shallow: bool,
) {
    let Ok(read) = std::fs::read_dir(abs_dir) else {
        return;
    };
    for entry in read.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let rel = rel_dir.join(&name);
        if ignores.dirs.iter().any(|d| d == name_str.as_ref()) {
            continue;
        }
        let abs = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&abs) else {
            continue;
        };
        let symlink_target = if metadata.is_symlink() {
            std::fs::read_link(&abs)
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        } else {
            None
        };
        let is_real_dir = metadata.is_dir() && !metadata.file_type().is_symlink();
        let is_vcs = ignores.vcs.iter().any(|d| d == name_str.as_ref());
        out.push((rel.clone(), metadata, symlink_target));
        if is_real_dir && !shallow {
            walk(&abs, &rel, ignores, out, is_vcs);
        }
    }
}

fn should_ignore(rel: &std::path::Path, ignores: &WatchIgnores) -> bool {
    let comps: Vec<_> = rel.components().collect();
    for (i, comp) in comps.iter().enumerate() {
        let s = comp.as_os_str().to_string_lossy();
        if ignores.dirs.iter().any(|d| d == s.as_ref()) {
            return true;
        }
        if ignores.vcs.iter().any(|d| d == s.as_ref()) {
            // Include the VCS directory and its direct children.
            return i + 2 < comps.len();
        }
    }
    false
}

pub(crate) struct WatchConfig {
    pub ignores: WatchIgnores,
    pub jj_events: bool,
}

pub(crate) fn load_watchman_config(root: &std::path::Path) -> WatchConfig {
    let jj_events = root.join(".jj").is_dir();
    let path = root.join(".watchmanconfig");
    let Ok(bytes) = std::fs::read(&path) else {
        return WatchConfig {
            ignores: WatchIgnores::default(),
            jj_events,
        };
    };
    let parsed: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(?e, path = ?path, "ignoring malformed .watchmanconfig");
            return WatchConfig {
                ignores: WatchIgnores::default(),
                jj_events,
            };
        }
    };
    let mut ignores = WatchIgnores::default();
    if let Some(dirs) = parsed.get("ignore_dirs").and_then(|v| v.as_array()) {
        ignores.dirs = dirs
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
    }
    if let Some(vcs) = parsed.get("ignore_vcs").and_then(|v| v.as_array()) {
        ignores.vcs = vcs
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
    }
    WatchConfig { ignores, jj_events }
}
