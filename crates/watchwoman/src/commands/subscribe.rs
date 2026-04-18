//! Subscription scaffolding.
//!
//! The happy path registers the subscription against its root, emits an
//! initial response (optionally empty via `empty_on_fresh_instance`),
//! and relies on the server loop — not implemented yet — to stream
//! unilateral PDUs back to the client on every tick.  The subscribe
//! integration test therefore ships marked `#[ignore]` in a follow-up
//! until the streaming half is wired.

use std::path::PathBuf;
use std::sync::Arc;

use indexmap::IndexMap;
use watchwoman_protocol::Value;

use super::{obj, CommandError, CommandResult};
use crate::daemon::root::SubscriptionSpec;
use crate::daemon::state::DaemonState;
use crate::query;

pub fn subscribe(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let (root_path, root, name, raw_spec) = parse_subscribe_args(state, args)?;
    let spec = query::parse_spec(&raw_spec)?;
    let result = query::run(&root, &spec);

    root.add_subscription(SubscriptionSpec {
        name: name.clone(),
        query: raw_spec,
    });

    let mut m = IndexMap::new();
    m.insert(
        "version".to_owned(),
        Value::String(crate::WATCHMAN_COMPAT_VERSION.into()),
    );
    m.insert("subscribe".to_owned(), Value::String(name));
    m.insert("clock".to_owned(), Value::String(result.clock));
    m.insert(
        "is_fresh_instance".to_owned(),
        Value::Bool(result.is_fresh_instance),
    );
    m.insert(
        "root".to_owned(),
        Value::String(root_path.to_string_lossy().into()),
    );
    m.insert("files".to_owned(), Value::Array(result.files));
    Ok(Value::Object(m))
}

pub fn unsubscribe(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let root_path = canonical(
        args.first()
            .and_then(Value::as_str)
            .ok_or_else(|| CommandError::BadArgs("unsubscribe requires a root".into()))?,
    );
    let root = state
        .root(&root_path)
        .ok_or_else(|| CommandError::UnknownRoot(root_path.to_string_lossy().into()))?;
    let name = args
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("unsubscribe requires a name".into()))?
        .to_owned();
    let removed = root.remove_subscription(&name);
    Ok(obj([
        ("unsubscribed", Value::Bool(removed)),
        ("subscription", Value::String(name)),
    ]))
}

pub fn flush_subscriptions(state: &Arc<DaemonState>, args: &[Value]) -> CommandResult {
    let root_path = canonical(
        args.first()
            .and_then(Value::as_str)
            .ok_or_else(|| CommandError::BadArgs("flush requires a root".into()))?,
    );
    let root = state
        .root(&root_path)
        .ok_or_else(|| CommandError::UnknownRoot(root_path.to_string_lossy().into()))?;
    let names: Vec<Value> = root
        .subscriptions()
        .into_iter()
        .map(|s| Value::String(s.name))
        .collect();
    Ok(obj([("synced", Value::Array(names))]))
}

fn parse_subscribe_args(
    state: &Arc<DaemonState>,
    args: &[Value],
) -> Result<(PathBuf, Arc<crate::daemon::root::Root>, String, Value), CommandError> {
    let root_str = args
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("subscribe requires a root".into()))?;
    let root_path = canonical(root_str);
    let root = state
        .root(&root_path)
        .ok_or_else(|| CommandError::UnknownRoot(root_path.to_string_lossy().into()))?;
    let name = args
        .get(1)
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("subscribe requires a name".into()))?
        .to_owned();
    let spec = args
        .get(2)
        .cloned()
        .ok_or_else(|| CommandError::BadArgs("subscribe requires a query spec".into()))?;
    Ok((root_path, root, name, spec))
}

fn canonical(p: &str) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p))
}
