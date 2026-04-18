use std::path::PathBuf;
use std::sync::Arc;

use watchwoman_protocol::Value;

use super::{obj, CommandError, CommandResult};
use crate::daemon::state::DaemonState;

pub fn state_enter(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let (root, name) = parse(state, args)?;
    root.asserted_states.write().insert(name.clone());
    Ok(obj([("state-enter", Value::String(name))]))
}

pub fn state_leave(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let (root, name) = parse(state, args)?;
    root.asserted_states.write().remove(&name);
    Ok(obj([("state-leave", Value::String(name))]))
}

fn parse(
    state: &Arc<DaemonState>,
    args: &[Value],
) -> Result<(Arc<crate::daemon::root::Root>, String), CommandError> {
    let path = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("expected a root path".into()))?;
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    let root = state
        .root(&canonical)
        .ok_or_else(|| CommandError::UnknownRoot(canonical.to_string_lossy().into()))?;
    let name = args
        .get(1)
        .and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Object(o) => o.get("name").and_then(Value::as_str).map(str::to_owned),
            _ => None,
        })
        .ok_or_else(|| CommandError::BadArgs("expected a state name".into()))?;
    Ok((root, name))
}
