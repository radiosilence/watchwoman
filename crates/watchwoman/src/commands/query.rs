use std::path::PathBuf;
use std::sync::Arc;

use indexmap::IndexMap;
use watchwoman_protocol::Value;

use super::{CommandError, CommandResult};
use crate::daemon::state::DaemonState;
use crate::query;

pub fn query(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let root_arg = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("query requires a root".into()))?;
    let root_path = canonical(root_arg);
    let root = state
        .root(&root_path)
        .ok_or_else(|| CommandError::UnknownRoot(root_path.to_string_lossy().into()))?;
    let raw = args
        .get(1)
        .ok_or_else(|| CommandError::BadArgs("query requires a spec object".into()))?;
    let spec = query::parse_spec(raw)?;
    let result = query::run(&root, &spec);
    Ok(query::run::result_to_pdu(&root_path, result))
}

pub fn find(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let root_arg = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("find requires a root".into()))?;
    let root_path = canonical(root_arg);
    let root = state
        .root(&root_path)
        .ok_or_else(|| CommandError::UnknownRoot(root_path.to_string_lossy().into()))?;
    let mut patterns = Vec::new();
    for a in &args[1..] {
        if let Some(s) = a.as_str() {
            patterns.push(s.to_owned());
        }
    }
    let mut spec_obj = IndexMap::new();
    if !patterns.is_empty() {
        spec_obj.insert(
            "glob".to_owned(),
            Value::Array(patterns.into_iter().map(Value::String).collect()),
        );
    }
    let spec = query::parse_spec(&Value::Object(spec_obj))?;
    let result = query::run(&root, &spec);
    Ok(query::run::result_to_pdu(&root_path, result))
}

pub fn since(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let root_arg = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("since requires a root".into()))?;
    let root_path = canonical(root_arg);
    let root = state
        .root(&root_path)
        .ok_or_else(|| CommandError::UnknownRoot(root_path.to_string_lossy().into()))?;
    let clock = args
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("since requires a clock".into()))?;
    let mut spec_obj = IndexMap::new();
    spec_obj.insert("since".to_owned(), Value::String(clock.to_owned()));
    let spec = query::parse_spec(&Value::Object(spec_obj))?;
    let result = query::run(&root, &spec);
    Ok(query::run::result_to_pdu(&root_path, result))
}

fn canonical(p: &str) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p))
}
