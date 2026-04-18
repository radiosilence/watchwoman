//! notify-backed filesystem watcher.  One watcher per root; coalesces
//! events into tick batches before the tree sees them.
//!
//! We intentionally do not stream every event individually — that's
//! watchman's most common source of "too many inotify watches" and
//! "recrawl" noise.  Batching at 5 ms matches watchman's default
//! settle period well enough for CLI tools.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ignore::WalkBuilder;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use super::root::{PathChange, Root, WatcherCommand};

const SETTLE: Duration = Duration::from_millis(5);
const MAX_BATCH: usize = 1024;

pub fn spawn(root: Arc<Root>, mut cmd_rx: mpsc::UnboundedReceiver<WatcherCommand>) {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<notify::Result<Event>>();

    let root_path = root.path.clone();
    let watcher_handle = tokio::task::spawn_blocking(move || {
        let mut watcher: RecommendedWatcher = match notify::recommended_watcher(move |res| {
            let _ = event_tx.send(res);
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(?e, "failed to create watcher");
                return;
            }
        };
        if let Err(e) = watcher.watch(&root_path, RecursiveMode::Recursive) {
            tracing::warn!(?e, "failed to watch root");
        }
        // Park the watcher thread — dropping this task's handle drops
        // the watcher, and we rely on cmd_rx shutdown to park.
        std::thread::park();
    });

    let root_for_events = root.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                cmd = cmd_rx.recv() => match cmd {
                    Some(WatcherCommand::Shutdown) | None => break,
                },
                first = event_rx.recv() => {
                    let Some(first) = first else { break };
                    let mut batch = Vec::with_capacity(8);
                    collect_event(&root_for_events, first, &mut batch);
                    let deadline = tokio::time::Instant::now() + SETTLE;
                    while batch.len() < MAX_BATCH {
                        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                        if remaining.is_zero() { break; }
                        match tokio::time::timeout(remaining, event_rx.recv()).await {
                            Ok(Some(next)) => collect_event(&root_for_events, next, &mut batch),
                            Ok(None) | Err(_) => break,
                        }
                    }
                    if !batch.is_empty() {
                        root_for_events.apply_changes(batch);
                    }
                }
            }
        }
        watcher_handle.abort();
    });
}

fn collect_event(root: &Root, event: notify::Result<Event>, out: &mut Vec<PathChange>) {
    let event = match event {
        Ok(ev) => ev,
        Err(e) => {
            tracing::debug!(?e, "notify error");
            return;
        }
    };
    for abs in &event.paths {
        let Some(rel) = abs.strip_prefix(&root.path).ok().map(PathBuf::from) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        if should_ignore(&rel) {
            continue;
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

pub(crate) fn initial_scan(root: &PathBuf) -> Vec<(PathBuf, std::fs::Metadata, Option<String>)> {
    // `ignore` integration is opt-in per watchman semantics — it surfaces
    // every file by default.  Tools that want gitignore filtering do so at
    // query time, not at scan time.  We only prune the obvious VCS /
    // build dirs via [`should_ignore`] below.
    let walker = WalkBuilder::new(root)
        .follow_links(false)
        .hidden(false)
        .git_ignore(false)
        .git_exclude(false)
        .git_global(false)
        .ignore(false)
        .parents(false)
        .build();
    let mut out = Vec::new();
    for entry in walker.flatten() {
        let abs = entry.path();
        let Some(rel) = abs.strip_prefix(root).ok().map(PathBuf::from) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        if should_ignore(&rel) {
            continue;
        }
        let Ok(metadata) = std::fs::symlink_metadata(abs) else {
            continue;
        };
        let symlink_target = if metadata.is_symlink() {
            std::fs::read_link(abs)
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        } else {
            None
        };
        out.push((rel, metadata, symlink_target));
    }
    out
}

fn should_ignore(rel: &std::path::Path) -> bool {
    // Mirror watchman's `ignore_dirs` default plus common VCS / build
    // output — saves millions of unnecessary entries on big trees.
    for component in rel.components() {
        let s = component.as_os_str().to_string_lossy();
        matches!(
            s.as_ref(),
            ".git" | ".hg" | ".svn" | "node_modules" | "target" | ".direnv" | ".venv"
        )
        .then_some(())
        .map(|_| true)
        .unwrap_or(false);
        if matches!(
            s.as_ref(),
            ".git" | ".hg" | ".svn" | "node_modules" | "target"
        ) {
            return true;
        }
    }
    false
}
