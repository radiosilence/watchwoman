//! Per-root state: file tree, clock, asserted states, subscriptions.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::sync::{broadcast, mpsc};

use super::clock::Clock;
use super::tree::{FileEntry, Tree};

/// Information published on every tick — used by subscriptions.
#[derive(Debug, Clone)]
pub struct TickEvent {
    pub tick: u64,
    pub changed: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct SubscriptionSpec {
    pub name: String,
    pub query: watchwoman_protocol::Value,
}

pub struct Root {
    pub path: PathBuf,
    pub root_number: u64,
    pub clock: Clock,
    pub tree: RwLock<Tree>,
    pub asserted_states: RwLock<HashSet<String>>,
    pub tick_tx: broadcast::Sender<TickEvent>,
    /// Channel the watcher thread uses to push raw notify events. Dropped
    /// (and thus closed) when the root is removed, which tears the
    /// watcher down automatically.
    pub watcher_cmd_tx: mpsc::UnboundedSender<WatcherCommand>,
    root_number_pool: Arc<AtomicU64>,
    subscriptions: RwLock<HashMap<String, SubscriptionSpec>>,
}

pub enum WatcherCommand {
    Shutdown,
}

impl Root {
    pub fn new(
        path: PathBuf,
        root_number: u64,
        root_number_pool: Arc<AtomicU64>,
        watcher_cmd_tx: mpsc::UnboundedSender<WatcherCommand>,
    ) -> Self {
        let (tx, _rx) = broadcast::channel(256);
        Self {
            path,
            root_number,
            clock: Clock::new(root_number),
            tree: RwLock::new(Tree::new()),
            asserted_states: RwLock::new(HashSet::new()),
            tick_tx: tx,
            watcher_cmd_tx,
            root_number_pool,
            subscriptions: RwLock::new(HashMap::new()),
        }
    }

    /// Apply a batch of path changes to the tree and bump the clock once.
    /// Returns the tick that covers this batch.
    pub fn apply_changes(&self, changes: Vec<PathChange>) -> u64 {
        let tick = self.clock.bump();
        let mut tree = self.tree.write();
        let mut changed_paths = Vec::with_capacity(changes.len());
        for change in changes {
            match change {
                PathChange::Upsert {
                    rel,
                    metadata,
                    symlink_target,
                } => {
                    let fresh = !tree.contains(&rel);
                    let mut entry = FileEntry::from_metadata(&metadata, symlink_target, tick);
                    entry.is_new = fresh;
                    // Preserve creation clock on updates.
                    if let Some(existing) = tree.get(&rel) {
                        entry.cclock = existing.cclock;
                    }
                    tree.insert(rel.clone(), entry);
                    changed_paths.push(rel);
                }
                PathChange::Remove { rel } => {
                    tree.update(&rel, |e| e.mark_gone(tick));
                    changed_paths.push(rel);
                }
            }
        }
        drop(tree);
        let _ = self.tick_tx.send(TickEvent {
            tick,
            changed: changed_paths,
        });
        tick
    }

    /// Seed the tree from an initial full scan. Same bookkeeping as
    /// [`Self::apply_changes`] but marks entries as non-new so the first
    /// subscription payload doesn't spam the client with "fresh" files.
    pub fn seed(&self, entries: Vec<(PathBuf, std::fs::Metadata, Option<String>)>) -> u64 {
        let tick = self.clock.bump();
        let mut tree = self.tree.write();
        for (rel, metadata, symlink_target) in entries {
            let mut entry = FileEntry::from_metadata(&metadata, symlink_target, tick);
            entry.is_new = false;
            tree.insert(rel, entry);
        }
        tick
    }

    pub fn clock_string(&self) -> String {
        self.clock.current_string()
    }

    pub fn add_subscription(&self, spec: SubscriptionSpec) {
        self.subscriptions.write().insert(spec.name.clone(), spec);
    }

    pub fn remove_subscription(&self, name: &str) -> bool {
        self.subscriptions.write().remove(name).is_some()
    }

    pub fn subscriptions(&self) -> Vec<SubscriptionSpec> {
        self.subscriptions.read().values().cloned().collect()
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = self.watcher_cmd_tx.send(WatcherCommand::Shutdown);
        // Return the root number so later watches can reuse it. Not
        // load-bearing; keeps `clock.root_number` stable across tests.
        let _ = self
            .root_number_pool
            .fetch_max(self.root_number, Ordering::AcqRel);
    }
}

#[derive(Debug)]
pub enum PathChange {
    Upsert {
        rel: PathBuf,
        metadata: std::fs::Metadata,
        symlink_target: Option<String>,
    },
    Remove {
        rel: PathBuf,
    },
}
