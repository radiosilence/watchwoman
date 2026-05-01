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
    "cmd-debug-contenthash",
    "cmd-debug-fsevents-inject-drop",
    "cmd-debug-get-asserted-states",
    "cmd-debug-get-subscriptions",
    "cmd-debug-kqueue-and-fsevents-recrawl",
    "cmd-debug-poll-for-settle",
    "cmd-debug-recrawl",
    "cmd-debug-root-status",
    "cmd-debug-set-parallel-crawl",
    "cmd-debug-set-subscriptions-paused",
    "cmd-debug-show-cursors",
    "cmd-debug-status",
    "cmd-debug-symlink-target-cache",
    "cmd-debug-watcher-info",
    "cmd-debug-watcher-info-clear",
    "cmd-find",
    "cmd-flush-subscriptions",
    "cmd-get-config",
    "cmd-get-log",
    "cmd-get-pid",
    "cmd-get-sockname",
    "cmd-global-log-level",
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
    "field-content.sha1hex",
    "field-ctime",
    "field-ctime_f",
    "field-ctime_ms",
    "field-ctime_ns",
    "field-ctime_us",
    "field-dev",
    "field-exists",
    "field-gid",
    "field-ino",
    "field-mode",
    "field-mtime",
    "field-mtime_f",
    "field-mtime_ms",
    "field-mtime_ns",
    "field-mtime_us",
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

/// The active watcher backend, advertised alongside [`CAPABILITIES`].
/// Real watchman publishes one of `watcher-fsevents` / `watcher-kqueue`
/// / `watcher-eden`; inotify isn't in their list but we'd rather tell
/// the truth than lie about the backend we actually use.
#[cfg(target_os = "macos")]
const WATCHER_CAPABILITY: &str = "watcher-fsevents";
#[cfg(target_os = "linux")]
const WATCHER_CAPABILITY: &str = "watcher-inotify";
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
const WATCHER_CAPABILITY: &str = "watcher-kqueue";

fn has_capability(name: &str) -> bool {
    CAPABILITIES.contains(&name) || name == WATCHER_CAPABILITY
}

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
                        let have = has_capability(name);
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
                        caps.insert(name.to_owned(), Value::Bool(has_capability(name)));
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
    let mut v: Vec<Value> = CAPABILITIES
        .iter()
        .map(|c| Value::String((*c).to_owned()))
        .collect();
    v.push(Value::String(WATCHER_CAPABILITY.to_owned()));
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
    let mut total_live: i64 = 0;
    let mut total_tombstones: i64 = 0;
    let mut total_subs: i64 = 0;
    let mut total_triggers: i64 = 0;
    let mut total_tree_bytes: u64 = 0;
    let mut roots: Vec<Value> = Vec::new();
    for entry in state.roots.iter() {
        let path = entry.key();
        let root = entry.value();
        let (num_files, live, tombstones, tree_bytes) = {
            let tree = root.tree.read();
            let n = tree.len();
            let live = tree.live_count();
            let tomb = tree.tombstone_count();
            // arena_bytes is exact: every per-root allocation goes
            // through the bumpalo arena (path keys, symlink targets,
            // HashMap backing array, FileEntry payloads), so this is
            // the per-root memory footprint not an estimate.
            let bytes = tree.arena_bytes() as u64;
            (n as i64, live as i64, tomb as i64, bytes)
        };
        total_files += num_files;
        total_live += live;
        total_tombstones += tombstones;
        total_tree_bytes = total_tree_bytes.saturating_add(tree_bytes);
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
        m.insert("live_files".into(), Value::Int(live));
        m.insert("tombstones".into(), Value::Int(tombstones));
        m.insert("tree_bytes_est".into(), Value::Int(tree_bytes as i64));
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

    let unaccounted = rss_bytes.saturating_sub(total_tree_bytes);
    let mut memory = IndexMap::new();
    memory.insert("rss_bytes".into(), Value::Int(rss_bytes as i64));
    memory.insert("tree_bytes_est".into(), Value::Int(total_tree_bytes as i64));
    memory.insert("unaccounted_bytes".into(), Value::Int(unaccounted as i64));
    memory.insert("live_entries".into(), Value::Int(total_live));
    memory.insert("tombstone_entries".into(), Value::Int(total_tombstones));
    memory.insert(
        "entry_size_bytes".into(),
        Value::Int(crate::daemon::tree::ENTRY_SIZE as i64),
    );

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
        // Real watchwoman semver, distinct from the watchman date-
        // stamped `compat_version` we hand back from the `version`
        // command for client-compat probes.  The CLI render slot
        // (`watchwoman {version}  (pid …)`) was reading this field
        // already; without it we printed a blank where the version
        // should be.
        ("version", Value::String(crate::WATCHWOMAN_VERSION.into())),
        (
            "compat_version",
            Value::String(crate::WATCHMAN_COMPAT_VERSION.into()),
        ),
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
        ("total_live_files", Value::Int(total_live)),
        ("total_tombstones", Value::Int(total_tombstones)),
        ("total_subscriptions", Value::Int(total_subs)),
        ("total_triggers", Value::Int(total_triggers)),
        ("memory", Value::Object(memory)),
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
    // Mirror the GC's stale threshold so the report reflects what the
    // daemon will actually reap.  A 0 threshold means stale reaping is
    // disabled — never mark a root "stale" in that mode.
    let threshold = crate::daemon::gc::stale_idle_secs() as i64;
    if threshold > 0 && idle_seconds >= threshold {
        "stale"
    } else {
        "idle"
    }
}

/// Read the daemon's own resource usage.  Returns `(rss_bytes,
/// user_cpu_ms, system_cpu_ms)`.
///
/// CPU times come from `getrusage(RUSAGE_SELF)` — those are cumulative
/// counters where the kernel-reported value is exactly what we want.
///
/// RSS comes from a platform-specific path: `getrusage`'s `ru_maxrss`
/// is the **peak** RSS the process has ever reached, monotonic for the
/// lifetime of the process.  Surfacing peak as "current memory" makes
/// the status report lie after a `watch-del-all` — operators see a
/// huge number that never drops and assume the daemon has leaked.  We
/// read the live resident set instead.
fn self_resource_usage() -> (u64, u64, u64) {
    use nix::sys::resource::{getrusage, UsageWho};
    let cpu = getrusage(UsageWho::RUSAGE_SELF).ok();
    let to_ms = |t: nix::sys::time::TimeVal| -> u64 {
        let secs = t.tv_sec().max(0) as u64;
        let us = t.tv_usec().max(0) as u64;
        secs.saturating_mul(1000) + us / 1000
    };
    let (user_ms, system_ms) = match cpu {
        Some(u) => (to_ms(u.user_time()), to_ms(u.system_time())),
        None => (0, 0),
    };
    let rss_bytes = current_rss_bytes().unwrap_or(0);
    (rss_bytes, user_ms, system_ms)
}

/// Current resident-set size in bytes, or `None` if the platform-
/// specific read failed.  Unlike `getrusage::ru_maxrss` this drops
/// when memory is released — exactly what `status` needs to surface.
#[cfg(target_os = "macos")]
fn current_rss_bytes() -> Option<u64> {
    // `task_info(MACH_TASK_BASIC_INFO)` is the canonical way to read
    // live RSS on macOS.  The struct layout matches `<mach/task_info.h>`
    // — three `mach_vm_size_t` followed by two `time_value_t` followed
    // by `policy_t` and `integer_t`.  Total size is 48 bytes, count is
    // 12 `natural_t` (u32) words.
    use std::mem::MaybeUninit;

    #[repr(C)]
    struct MachTaskBasicInfo {
        virtual_size: u64,
        resident_size: u64,
        resident_size_max: u64,
        user_time_seconds: i32,
        user_time_microseconds: i32,
        system_time_seconds: i32,
        system_time_microseconds: i32,
        policy: i32,
        suspend_count: i32,
    }

    const MACH_TASK_BASIC_INFO: u32 = 20;
    const MACH_TASK_BASIC_INFO_COUNT: u32 =
        (std::mem::size_of::<MachTaskBasicInfo>() / std::mem::size_of::<u32>()) as u32;

    extern "C" {
        fn mach_task_self() -> u32;
        fn task_info(
            target_task: u32,
            flavor: u32,
            task_info_out: *mut i32,
            task_info_count: *mut u32,
        ) -> i32;
    }

    let mut info = MaybeUninit::<MachTaskBasicInfo>::uninit();
    let mut count = MACH_TASK_BASIC_INFO_COUNT;
    // SAFETY: `mach_task_self()` returns this process's task port,
    // which `task_info` always accepts.  The buffer is sized to match
    // `MACH_TASK_BASIC_INFO_COUNT` words, the size the flavor writes.
    let kr = unsafe {
        task_info(
            mach_task_self(),
            MACH_TASK_BASIC_INFO,
            info.as_mut_ptr() as *mut i32,
            &mut count,
        )
    };
    if kr != 0 {
        return None;
    }
    // SAFETY: `task_info` returned KERN_SUCCESS, so the buffer is now
    // fully initialised in the layout we declared.
    let info = unsafe { info.assume_init() };
    Some(info.resident_size)
}

#[cfg(target_os = "linux")]
fn current_rss_bytes() -> Option<u64> {
    // /proc/self/statm columns are page counts: size resident shared
    // text lib data dt.  Resident is column 2.
    let s = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = s.split_whitespace().nth(1)?.parse().ok()?;
    let page_size = page_size_bytes()?;
    Some(pages.saturating_mul(page_size))
}

#[cfg(target_os = "linux")]
fn page_size_bytes() -> Option<u64> {
    extern "C" {
        fn getpagesize() -> i32;
    }
    // SAFETY: getpagesize() is a leaf syscall wrapper; no preconditions.
    let n = unsafe { getpagesize() };
    if n > 0 {
        Some(n as u64)
    } else {
        None
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn current_rss_bytes() -> Option<u64> {
    // No portable live-RSS read on other unixes.  status will report
    // 0 on those platforms rather than the misleading peak.
    None
}
