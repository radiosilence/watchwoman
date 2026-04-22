use std::sync::Arc;

use indexmap::IndexMap;
use watchwoman_protocol::Value;

use super::{obj, CommandError, CommandResult};
use crate::daemon::state::DaemonState;

const CAPABILITIES: &[&str] = &[
    "bser-v2",
    "clock-sync-timeout",
    "cmd-clock",
    "cmd-debug-ageout",
    "cmd-debug-poll-for-settle",
    "cmd-debug-recrawl",
    "cmd-debug-show-cursors",
    "cmd-find",
    "cmd-flush-subscriptions",
    "cmd-get-config",
    "cmd-get-pid",
    "cmd-get-sockname",
    "cmd-list-capabilities",
    "cmd-log",
    "cmd-log-level",
    "cmd-query",
    "cmd-shutdown-server",
    "cmd-since",
    "cmd-status",
    "cmd-state-enter",
    "cmd-state-leave",
    "cmd-subscribe",
    "cmd-trigger",
    "cmd-trigger-del",
    "cmd-trigger-list",
    "cmd-unsubscribe",
    "cmd-version",
    "cmd-watch",
    "cmd-watch-del",
    "cmd-watch-del-all",
    "cmd-watch-list",
    "cmd-watch-project",
    "dedup_results",
    "field-cclock",
    "field-ctime",
    "field-ctime_ms",
    "field-ctime_ns",
    "field-dev",
    "field-exists",
    "field-gid",
    "field-ino",
    "field-mode",
    "field-mtime",
    "field-mtime_ms",
    "field-mtime_ns",
    "field-name",
    "field-new",
    "field-nlink",
    "field-oclock",
    "field-size",
    "field-symlink_target",
    "field-type",
    "field-uid",
    "glob_generator",
    "path_generator",
    "relative_root",
    "scm-git",
    "scm-hg",
    "scm-since",
    "suffix-set",
    "term-allof",
    "term-anyof",
    "term-dirname",
    "term-empty",
    "term-exists",
    "term-false",
    "term-idirname",
    "term-imatch",
    "term-iname",
    "term-ipcre",
    "term-match",
    "term-name",
    "term-not",
    "term-pcre",
    "term-since",
    "term-size",
    "term-suffix",
    "term-true",
    "term-type",
    "wildmatch",
    "wildmatch-multislash",
];

pub fn get_sockname(state: &Arc<DaemonState>) -> CommandResult {
    let s = state.sock_path.to_string_lossy().to_string();
    Ok(obj([
        (
            "version",
            Value::String(crate::WATCHMAN_COMPAT_VERSION.into()),
        ),
        ("sockname", Value::String(s.clone())),
        ("unix_domain", Value::String(s)),
    ]))
}

pub fn get_pid() -> CommandResult {
    Ok(obj([("pid", Value::Int(std::process::id() as i64))]))
}

pub fn version(args: &[Value]) -> CommandResult {
    let mut caps = IndexMap::new();
    let mut error: Option<String> = None;
    if let Some(spec) = args.first() {
        if let Some(obj) = spec.as_object() {
            if let Some(required) = obj.get("required").and_then(Value::as_array) {
                for c in required {
                    if let Some(name) = c.as_str() {
                        let have = CAPABILITIES.contains(&name);
                        caps.insert(name.to_owned(), Value::Bool(have));
                        if !have && error.is_none() {
                            error = Some(format!("required capability `{name}` is not supported"));
                        }
                    }
                }
            }
            if let Some(optional) = obj.get("optional").and_then(Value::as_array) {
                for c in optional {
                    if let Some(name) = c.as_str() {
                        caps.insert(name.to_owned(), Value::Bool(CAPABILITIES.contains(&name)));
                    }
                }
            }
        }
    }

    let mut out = IndexMap::new();
    out.insert(
        "version".into(),
        Value::String(crate::WATCHMAN_COMPAT_VERSION.into()),
    );
    out.insert(
        "buildinfo".into(),
        Value::String(format!("watchwoman {}", crate::WATCHWOMAN_VERSION)),
    );
    if !caps.is_empty() {
        out.insert("capabilities".into(), Value::Object(caps));
    }
    if let Some(msg) = error {
        out.insert("error".into(), Value::String(msg));
    }
    Ok(Value::Object(out))
}

pub fn list_capabilities() -> CommandResult {
    let v: Vec<Value> = CAPABILITIES
        .iter()
        .map(|c| Value::String((*c).to_owned()))
        .collect();
    Ok(obj([("capabilities", Value::Array(v))]))
}

pub fn get_config(args: &[Value]) -> CommandResult {
    // If a root is supplied and it has a `.watchmanconfig`, return
    // those contents verbatim. Otherwise return an empty config.
    if let Some(path) = args.first().and_then(Value::as_str) {
        let file = std::path::PathBuf::from(path).join(".watchmanconfig");
        if let Ok(bytes) = std::fs::read(&file) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                return Ok(obj([("config", json_to_value(v))]));
            }
        }
    }
    Ok(obj([("config", Value::Object(IndexMap::new()))]))
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

pub fn get_log() -> CommandResult {
    // Watchwoman intentionally doesn't maintain an in-daemon log;
    // tracing goes to stderr. Return an empty log for shape parity.
    Ok(obj([("log", Value::Array(vec![]))]))
}

pub fn log_level(_args: &[Value]) -> CommandResult {
    Ok(obj([("log_level", Value::String("warn".into()))]))
}

pub fn log(args: &[Value]) -> CommandResult {
    let level = args
        .first()
        .and_then(Value::as_str)
        .unwrap_or("info")
        .to_owned();
    let msg = args
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("log message missing".into()))?;
    tracing::info!(level, %msg, "client log");
    Ok(obj([("logged", Value::Bool(true))]))
}

/// Comprehensive daemon status: uptime, memory, CPU, every watched
/// root with its file count and idle time, plus the last 64 GC reaps.
/// Returned as structured JSON; the CLI side renders it as a human
/// report when `--json` isn't set.
pub fn status(state: &Arc<DaemonState>) -> CommandResult {
    let (rss_bytes, user_cpu_ms, system_cpu_ms) = self_resource_usage();

    let mut total_files: i64 = 0;
    let mut total_subs: i64 = 0;
    let mut total_triggers: i64 = 0;
    let mut roots: Vec<Value> = Vec::new();
    for entry in state.roots.iter() {
        let path = entry.key();
        let root = entry.value();
        let num_files = root.tree.read().len() as i64;
        total_files += num_files;
        let subs = root.subscription_count() as i64;
        let triggers = root.trigger_count() as i64;
        total_subs += subs;
        total_triggers += triggers;
        let exists = std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false);
        let missing_ticks = root.missing_ticks() as i64;
        let idle = root.idle_seconds() as i64;
        let health = classify_health(exists, missing_ticks, idle, subs, triggers);

        let mut m = IndexMap::new();
        m.insert(
            "path".into(),
            Value::String(path.to_string_lossy().into_owned()),
        );
        m.insert("num_files".into(), Value::Int(num_files));
        m.insert("clock".into(), Value::String(root.clock_string()));
        m.insert("subscriptions".into(), Value::Int(subs));
        m.insert("triggers".into(), Value::Int(triggers));
        m.insert("idle_seconds".into(), Value::Int(idle));
        m.insert(
            "registered_at_unix".into(),
            Value::Int(root.registered_at() as i64),
        );
        m.insert("exists".into(), Value::Bool(exists));
        m.insert("missing_ticks".into(), Value::Int(missing_ticks));
        m.insert("health".into(), Value::String(health.into()));
        roots.push(Value::Object(m));
    }

    let reaped: Vec<Value> = state
        .reap_log()
        .into_iter()
        .map(|e| {
            let mut m = IndexMap::new();
            m.insert(
                "path".into(),
                Value::String(e.path.to_string_lossy().into_owned()),
            );
            m.insert("reason".into(), Value::String(e.reason.as_str().into()));
            m.insert("at_unix".into(), Value::Int(e.at_unix as i64));
            Value::Object(m)
        })
        .collect();

    Ok(obj([
        ("pid", Value::Int(std::process::id() as i64)),
        (
            "sockname",
            Value::String(state.sock_path.to_string_lossy().into_owned()),
        ),
        ("uptime_seconds", Value::Int(state.uptime_seconds() as i64)),
        ("started_at_unix", Value::Int(state.started_at_unix as i64)),
        ("rss_bytes", Value::Int(rss_bytes as i64)),
        ("user_cpu_ms", Value::Int(user_cpu_ms as i64)),
        ("system_cpu_ms", Value::Int(system_cpu_ms as i64)),
        ("total_tracked_files", Value::Int(total_files)),
        ("total_subscriptions", Value::Int(total_subs)),
        ("total_triggers", Value::Int(total_triggers)),
        ("roots", Value::Array(roots)),
        ("reaped", Value::Array(reaped)),
    ]))
}

fn classify_health(
    exists: bool,
    missing_ticks: i64,
    idle_seconds: i64,
    subs: i64,
    triggers: i64,
) -> &'static str {
    if !exists {
        return if missing_ticks >= 2 {
            "dead"
        } else {
            "missing"
        };
    }
    if subs > 0 || triggers > 0 {
        return "active";
    }
    // Mirror the GC's stale threshold — 14 days — but the report is
    // informational: the daemon still reaps based on its own timer.
    const STALE_DAYS: i64 = 14;
    if idle_seconds >= STALE_DAYS * 24 * 3600 {
        "stale"
    } else {
        "idle"
    }
}

/// Read the daemon's own resource usage from `getrusage(RUSAGE_SELF)`.
/// Returns `(rss_bytes, user_cpu_ms, system_cpu_ms)`.  macOS reports
/// `ru_maxrss` in bytes; Linux in kilobytes — normalise both to bytes.
fn self_resource_usage() -> (u64, u64, u64) {
    use nix::sys::resource::{getrusage, UsageWho};
    let Ok(usage) = getrusage(UsageWho::RUSAGE_SELF) else {
        return (0, 0, 0);
    };
    let raw_rss = usage.max_rss();
    let rss_bytes = if cfg!(target_os = "macos") {
        raw_rss.max(0) as u64
    } else {
        (raw_rss.max(0) as u64).saturating_mul(1024)
    };
    let user = usage.user_time();
    let system = usage.system_time();
    let to_ms = |t: nix::sys::time::TimeVal| -> u64 {
        let secs = t.tv_sec().max(0) as u64;
        let us = t.tv_usec().max(0) as u64;
        secs.saturating_mul(1000) + us / 1000
    };
    (rss_bytes, to_ms(user), to_ms(system))
}
