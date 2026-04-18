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

/// Installed trigger: fork+exec command on each tick where the query
/// matches any changed file.
#[derive(Debug, Clone)]
pub struct Trigger {
    pub name: String,
    pub command: Vec<String>,
    pub expression: watchwoman_protocol::Value,
    pub append_files: bool,
    pub stdin: Option<TriggerStdin>,
    pub max_files_stdin: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
pub enum TriggerStdin {
    /// Filenames, one per line, newline-terminated.
    NamePerLine,
    /// A JSON array of `{"name": "..."}` objects.
    JsonName,
}

impl Trigger {
    /// Build the watchman-style query spec this trigger should run on
    /// every tick. `since_clock` fences the query so the trigger only
    /// fires for newly changed files.
    pub fn query_spec(
        &self,
        since_clock: &str,
    ) -> Result<watchwoman_protocol::Value, crate::commands::CommandError> {
        use watchwoman_protocol::Value;
        let mut spec = indexmap::IndexMap::new();
        spec.insert("expression".into(), self.expression.clone());
        spec.insert(
            "fields".into(),
            Value::Array(vec![Value::String("name".into())]),
        );
        spec.insert("since".into(), Value::String(since_clock.to_owned()));
        Ok(Value::Object(spec))
    }
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
    /// Directory the daemon persists per-root state into (triggers).
    /// `None` means "no durability" (tests, ephemeral daemons).
    state_dir: Option<PathBuf>,
    /// `ignore_dirs` from the root's `.watchmanconfig`.  Path
    /// components matching any of these (exact basename) are skipped
    /// in both the initial scan and subsequent fs events.
    pub ignore_dirs: Vec<String>,
    root_number_pool: Arc<AtomicU64>,
    subscriptions: RwLock<HashMap<String, SubscriptionSpec>>,
    triggers: RwLock<HashMap<String, Trigger>>,
    cursors: RwLock<HashMap<String, u64>>,
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
        state_dir: Option<PathBuf>,
        ignore_dirs: Vec<String>,
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
            state_dir,
            ignore_dirs,
            root_number_pool,
            subscriptions: RwLock::new(HashMap::new()),
            triggers: RwLock::new(HashMap::new()),
            cursors: RwLock::new(HashMap::new()),
        }
    }

    /// Return the tick a named cursor points at, or 0 if it's new.
    /// Watchman's `n:cursor` clocks are "file-scoped tick memories": a
    /// query with `since: "n:foo"` filters to files observed after the
    /// cursor's last tick, and the query's completion advances the
    /// cursor.
    pub fn cursor_tick(&self, name: &str) -> u64 {
        self.cursors.read().get(name).copied().unwrap_or(0)
    }

    pub fn set_cursor(&self, name: &str, tick: u64) {
        self.cursors.write().insert(name.to_owned(), tick);
    }

    pub fn cursors(&self) -> Vec<(String, u64)> {
        self.cursors
            .read()
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect()
    }

    pub fn install_trigger(&self, t: Trigger) {
        self.triggers.write().insert(t.name.clone(), t);
        self.persist_triggers();
    }

    pub fn remove_trigger(&self, name: &str) -> bool {
        let removed = self.triggers.write().remove(name).is_some();
        if removed {
            self.persist_triggers();
        }
        removed
    }

    pub fn list_triggers(&self) -> Vec<Trigger> {
        self.triggers.read().values().cloned().collect()
    }

    pub fn has_trigger(&self, name: &str) -> bool {
        self.triggers.read().contains_key(name)
    }

    /// Serialise the installed triggers to
    /// `<state_dir>/roots/<root-slug>/triggers.json`.  Errors are
    /// logged and ignored — durability is an optimisation, not a
    /// correctness requirement.
    fn persist_triggers(&self) {
        let Some(dir) = self.trigger_persist_dir() else {
            return;
        };
        if let Err(e) = std::fs::create_dir_all(&dir) {
            tracing::warn!(?e, path = ?dir, "can't create trigger state dir");
            return;
        }
        let items: Vec<_> = self
            .triggers
            .read()
            .values()
            .map(persisted::PersistedTrigger::from_trigger)
            .collect();
        let path = dir.join("triggers.json");
        let tmp = dir.join("triggers.json.tmp");
        let bytes = match serde_json::to_vec_pretty(&items) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(?e, "can't serialise triggers");
                return;
            }
        };
        if std::fs::write(&tmp, &bytes).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    /// Load persisted triggers from disk on root register.
    pub fn load_persisted_triggers(&self) {
        let Some(dir) = self.trigger_persist_dir() else {
            return;
        };
        let path = dir.join("triggers.json");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let items: Vec<persisted::PersistedTrigger> = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(?e, path = ?path, "ignoring malformed triggers.json");
                return;
            }
        };
        let mut triggers = self.triggers.write();
        for pt in items {
            let t = pt.into_trigger();
            triggers.insert(t.name.clone(), t);
        }
    }

    fn trigger_persist_dir(&self) -> Option<PathBuf> {
        let state = self.state_dir.as_ref()?;
        let slug = path_slug(&self.path);
        Some(state.join("roots").join(slug))
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

/// Stable directory slug for a root path — ASCII-safe, no slashes.
fn path_slug(p: &std::path::Path) -> String {
    let s = p.to_string_lossy();
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

mod persisted {
    use super::{Trigger, TriggerStdin};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    pub(super) struct PersistedTrigger {
        pub name: String,
        pub command: Vec<String>,
        pub expression: serde_json::Value,
        #[serde(default)]
        pub append_files: bool,
        pub stdin: Option<String>,
        pub max_files_stdin: Option<usize>,
    }

    impl PersistedTrigger {
        pub fn from_trigger(t: &Trigger) -> Self {
            Self {
                name: t.name.clone(),
                command: t.command.clone(),
                expression: value_to_json(&t.expression),
                append_files: t.append_files,
                stdin: t.stdin.map(|s| match s {
                    TriggerStdin::NamePerLine => "NAME_PER_LINE".into(),
                    TriggerStdin::JsonName => "json".into(),
                }),
                max_files_stdin: t.max_files_stdin,
            }
        }

        pub fn into_trigger(self) -> Trigger {
            Trigger {
                name: self.name,
                command: self.command,
                expression: json_to_value(self.expression),
                append_files: self.append_files,
                stdin: match self.stdin.as_deref() {
                    Some("NAME_PER_LINE") => Some(TriggerStdin::NamePerLine),
                    Some("json") | Some("JSON") => Some(TriggerStdin::JsonName),
                    _ => None,
                },
                max_files_stdin: self.max_files_stdin,
            }
        }
    }

    fn value_to_json(v: &watchwoman_protocol::Value) -> serde_json::Value {
        use serde_json::{Number, Value as J};
        use watchwoman_protocol::Value;
        match v {
            Value::Null => J::Null,
            Value::Bool(b) => J::Bool(*b),
            Value::Int(i) => J::Number(Number::from(*i)),
            Value::Real(f) => Number::from_f64(*f).map(J::Number).unwrap_or(J::Null),
            Value::String(s) => J::String(s.clone()),
            Value::Bytes(b) => J::String(String::from_utf8_lossy(b).into_owned()),
            Value::Array(a) => J::Array(a.iter().map(value_to_json).collect()),
            Value::Object(o) => {
                let mut m = serde_json::Map::new();
                for (k, val) in o {
                    m.insert(k.clone(), value_to_json(val));
                }
                J::Object(m)
            }
            Value::Template { keys, rows } => {
                let mut out = Vec::with_capacity(rows.len());
                for row in rows {
                    let mut m = serde_json::Map::new();
                    for (k, val) in keys.iter().zip(row.iter()) {
                        m.insert(k.clone(), value_to_json(val));
                    }
                    out.push(J::Object(m));
                }
                J::Array(out)
            }
        }
    }

    fn json_to_value(v: serde_json::Value) -> watchwoman_protocol::Value {
        use indexmap::IndexMap;
        use serde_json::Value as J;
        use watchwoman_protocol::Value;
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
}
