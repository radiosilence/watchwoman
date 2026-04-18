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

pub fn get_config(_args: &[Value]) -> CommandResult {
    Ok(obj([("config", Value::Object(IndexMap::new()))]))
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
