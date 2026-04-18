use std::path::PathBuf;
use std::sync::Arc;

use watchwoman_protocol::Value;

use super::{obj, CommandError, CommandResult};
use crate::daemon::state::DaemonState;

pub fn clock(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let path = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("expected a root path".into()))?;
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    let root = state
        .root(&canonical)
        .ok_or_else(|| CommandError::UnknownRoot(canonical.to_string_lossy().into()))?;
    // `sync_timeout` is honoured by bumping the clock so the next
    // response is strictly ahead of anything the caller observed; we
    // don't yet block waiting for the kernel to drain.
    let _ = root.clock.bump();
    Ok(obj([("clock", Value::String(root.clock_string()))]))
}
