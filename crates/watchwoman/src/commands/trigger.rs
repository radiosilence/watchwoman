//! Triggers: fork-and-exec on file change.
//!
//! Watchman-style: install a trigger with a name + argv command + a
//! query expression. On every tick where the expression matches any
//! changed files, we `execvp` the command with the matched filenames
//! appended when `append_files` is set, or piped on stdin when
//! `"stdin": "NAME_PER_LINE"`.
//!
//! Intentionally not persistent: watchman stores triggers in a
//! `state` file so they survive daemon restart. We punt on that — the
//! daemon auto-spawns on first use, so a stale trigger from last
//! session is mostly a footgun anyway. Tooling that needs durable
//! triggers can reinstall them on startup.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use indexmap::IndexMap;
use watchwoman_protocol::Value;

use super::{obj, CommandError, CommandResult};
use crate::daemon::root::{Root, Trigger, TriggerStdin};
use crate::daemon::state::DaemonState;
use crate::query;

pub fn trigger(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let (root_path, root, spec) = parse_trigger_spec(state, args)?;
    let parsed = parse_trigger(&spec)?;
    let name = parsed.name.clone();
    root.install_trigger(parsed.clone());
    spawn_trigger_loop(root.clone(), root_path, parsed);
    Ok(obj([("trigger", Value::String(name))]))
}

pub fn trigger_list(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let root = resolve_root(state, args)?;
    let triggers: Vec<Value> = root
        .list_triggers()
        .into_iter()
        .map(trigger_to_value)
        .collect();
    Ok(obj([("triggers", Value::Array(triggers))]))
}

pub fn trigger_del(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let root = resolve_root(state, args)?;
    let name = args
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("trigger-del requires a name".into()))?;
    let removed = root.remove_trigger(name);
    Ok(obj([
        ("deleted", Value::Int(if removed { 1 } else { 0 })),
        ("trigger", Value::String(name.to_owned())),
    ]))
}

fn spawn_trigger_loop(root: Arc<Root>, root_path: PathBuf, trigger: Trigger) {
    let rx = root.tick_tx.subscribe();
    let start_tick = root.clock.current_tick();
    tokio::runtime::Handle::current().spawn(async move {
        run_trigger(rx, start_tick, root, root_path, trigger).await;
    });
}

async fn run_trigger(
    mut rx: tokio::sync::broadcast::Receiver<crate::daemon::root::TickEvent>,
    start_tick: u64,
    root: Arc<Root>,
    root_path: PathBuf,
    trigger: Trigger,
) {
    let mut last_tick = start_tick;
    loop {
        let ev = match rx.recv().await {
            Ok(ev) => ev,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        };
        if !root.has_trigger(&trigger.name) {
            break;
        }
        if ev.tick <= last_tick {
            continue;
        }
        let spec = match trigger.query_spec(&root.clock.encode(last_tick)) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(?e, trigger = %trigger.name, "invalid trigger query spec");
                break;
            }
        };
        let parsed = match query::parse_spec(&spec) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(?e, trigger = %trigger.name, "bad trigger expression");
                break;
            }
        };
        let result = query::run(&root, &parsed);
        last_tick = ev.tick;

        let filenames = collect_names(&result.files);
        if filenames.is_empty() {
            continue;
        }

        if let Err(e) = fire(&trigger, &root_path, &filenames) {
            tracing::warn!(?e, trigger = %trigger.name, "trigger command failed");
        }
    }
}

fn collect_names(files: &[Value]) -> Vec<String> {
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        let name = match f {
            Value::String(s) => Some(s.clone()),
            Value::Object(o) => o.get("name").and_then(Value::as_str).map(str::to_owned),
            _ => None,
        };
        if let Some(n) = name {
            out.push(n);
        }
    }
    out
}

fn fire(trigger: &Trigger, root: &PathBuf, filenames: &[String]) -> std::io::Result<()> {
    if trigger.command.is_empty() {
        return Ok(());
    }
    let (exe, base_args) = trigger.command.split_first().unwrap();
    let mut cmd = Command::new(exe);
    cmd.current_dir(root).args(base_args);

    if trigger.append_files {
        let cap = trigger.max_files_stdin.unwrap_or(filenames.len());
        for name in filenames.iter().take(cap) {
            cmd.arg(name);
        }
    }

    match &trigger.stdin {
        Some(TriggerStdin::NamePerLine) => {
            cmd.stdin(std::process::Stdio::piped());
        }
        Some(TriggerStdin::JsonName) => {
            cmd.stdin(std::process::Stdio::piped());
        }
        None => {
            cmd.stdin(std::process::Stdio::null());
        }
    }

    let mut child = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    if let Some(mut stdin_pipe) = child.stdin.take() {
        use std::io::Write as _;
        let cap = trigger.max_files_stdin.unwrap_or(filenames.len());
        match trigger.stdin {
            Some(TriggerStdin::NamePerLine) => {
                for name in filenames.iter().take(cap) {
                    let _ = writeln!(stdin_pipe, "{name}");
                }
            }
            Some(TriggerStdin::JsonName) => {
                let rows: Vec<serde_json::Value> = filenames
                    .iter()
                    .take(cap)
                    .map(|n| serde_json::json!({ "name": n }))
                    .collect();
                let _ = serde_json::to_writer(&mut stdin_pipe, &rows);
            }
            None => {}
        }
    }
    // Fire-and-forget by default — watchman's triggers run
    // detached. Dropping the Child sets SIGCHLD cleanup to the OS.
    std::mem::drop(child);
    Ok(())
}

fn parse_trigger_spec(
    state: &Arc<DaemonState>,
    args: &[Value],
) -> Result<(PathBuf, Arc<Root>, Value), CommandError> {
    let root_str = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("trigger requires a root".into()))?;
    let root_path = canonical(root_str);
    let root = state
        .root(&root_path)
        .ok_or_else(|| CommandError::UnknownRoot(root_path.to_string_lossy().into()))?;
    let spec = args
        .get(1)
        .cloned()
        .ok_or_else(|| CommandError::BadArgs("trigger requires a spec".into()))?;
    Ok((root_path, root, spec))
}

fn parse_trigger(spec: &Value) -> Result<Trigger, CommandError> {
    let obj = spec
        .as_object()
        .ok_or_else(|| CommandError::BadArgs("trigger spec must be an object".into()))?;
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("trigger.name is required".into()))?
        .to_owned();
    let command = obj
        .get("command")
        .and_then(Value::as_array)
        .ok_or_else(|| CommandError::BadArgs("trigger.command array is required".into()))?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    let expression = obj
        .get("expression")
        .cloned()
        .unwrap_or(Value::String("true".into()));
    let append_files = obj
        .get("append_files")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let stdin = match obj.get("stdin") {
        Some(Value::String(s)) if s == "NAME_PER_LINE" => Some(TriggerStdin::NamePerLine),
        Some(Value::String(s)) if s == "json" || s == "JSON" => Some(TriggerStdin::JsonName),
        Some(_) | None => None,
    };
    let max_files_stdin = obj
        .get("max_files_stdin")
        .and_then(Value::as_i64)
        .map(|n| n.max(0) as usize);
    Ok(Trigger {
        name,
        command,
        expression,
        append_files,
        stdin,
        max_files_stdin,
    })
}

fn resolve_root(state: &Arc<DaemonState>, args: &[Value]) -> Result<Arc<Root>, CommandError> {
    let root_str = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("trigger command requires a root".into()))?;
    let root_path = canonical(root_str);
    state
        .root(&root_path)
        .ok_or_else(|| CommandError::UnknownRoot(root_path.to_string_lossy().into()))
}

fn canonical(p: &str) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p))
}

fn trigger_to_value(t: Trigger) -> Value {
    let mut m = IndexMap::new();
    m.insert("name".into(), Value::String(t.name));
    m.insert(
        "command".into(),
        Value::Array(t.command.into_iter().map(Value::String).collect()),
    );
    m.insert("expression".into(), t.expression);
    m.insert("append_files".into(), Value::Bool(t.append_files));
    if let Some(s) = t.stdin {
        m.insert(
            "stdin".into(),
            Value::String(match s {
                TriggerStdin::NamePerLine => "NAME_PER_LINE".into(),
                TriggerStdin::JsonName => "json".into(),
            }),
        );
    }
    if let Some(n) = t.max_files_stdin {
        m.insert("max_files_stdin".into(), Value::Int(n as i64));
    }
    Value::Object(m)
}
