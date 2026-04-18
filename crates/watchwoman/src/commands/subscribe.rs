//! Subscriptions.
//!
//! `subscribe` installs the subscription on the root and spawns a push
//! task keyed to the caller's session.  The push task watches the
//! root's tick broadcast, runs the subscription's query against
//! changes since the last delivered tick, and enqueues a unilateral
//! PDU onto the session's mpsc — which the writer loop turns into
//! bytes on the wire.
//!
//! When the client disconnects, the session's mpsc closes, `send`
//! fails, and the push task exits.  No explicit shutdown plumbing
//! required.

use std::path::PathBuf;
use std::sync::Arc;

use indexmap::IndexMap;
use watchwoman_protocol::Value;

use super::{obj, CommandError, CommandResult};
use crate::daemon::root::{Root, SubscriptionSpec};
use crate::daemon::session::Session;
use crate::daemon::state::DaemonState;
use crate::query;

pub fn subscribe(state: &Arc<DaemonState>, session: &Session, args: &[Value]) -> CommandResult {
    let (root_path, root, name, raw_spec) = parse_subscribe_args(state, args)?;
    let parsed = query::parse_spec(&raw_spec)?;
    let initial = query::run(&root, &parsed);

    root.add_subscription(SubscriptionSpec {
        name: name.clone(),
        query: raw_spec.clone(),
    });

    // Subscribe to the tick broadcast synchronously so we don't miss
    // any events between returning the initial PDU and the push task
    // actually polling. The starting tick fence is also captured here,
    // before we run the initial query, for the same reason.
    let rx = root.tick_tx.subscribe();
    let start_tick = root.clock.current_tick();

    // The command handler itself is sync (spawn_blocking), so we grab
    // the current runtime handle explicitly to spawn the push loop.
    let runtime = tokio::runtime::Handle::current();
    let push_root = root.clone();
    let push_session = session.clone();
    let push_name = name.clone();
    let push_path = root_path.clone();
    let push_raw = raw_spec;
    runtime.spawn(async move {
        run_push_loop(
            rx,
            start_tick,
            push_root,
            push_session,
            push_name,
            push_path,
            push_raw,
        )
        .await;
    });

    let mut m = IndexMap::new();
    m.insert(
        "version".into(),
        Value::String(crate::WATCHMAN_COMPAT_VERSION.into()),
    );
    m.insert("subscribe".into(), Value::String(name));
    m.insert("clock".into(), Value::String(initial.clock));
    m.insert(
        "is_fresh_instance".into(),
        Value::Bool(initial.is_fresh_instance),
    );
    m.insert(
        "root".into(),
        Value::String(root_path.to_string_lossy().into()),
    );
    m.insert("files".into(), Value::Array(initial.files));
    Ok(Value::Object(m))
}

async fn run_push_loop(
    mut rx: tokio::sync::broadcast::Receiver<crate::daemon::root::TickEvent>,
    start_tick: u64,
    root: Arc<Root>,
    session: Session,
    name: String,
    root_path: PathBuf,
    raw_spec: Value,
) {
    let mut last_tick = start_tick;

    loop {
        let ev = match rx.recv().await {
            Ok(ev) => ev,
            // Lagging receivers can skip ticks; that's fine — the query
            // will re-scan from `last_tick`.
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        };
        if session.is_closed() {
            break;
        }
        if ev.tick <= last_tick {
            continue;
        }

        let mut since_spec = match raw_spec.as_object() {
            Some(o) => o.clone(),
            None => IndexMap::new(),
        };
        let tick_clock = root.clock.encode(last_tick);
        since_spec.insert("since".into(), Value::String(tick_clock));
        let spec_val = Value::Object(since_spec);

        let parsed = match query::parse_spec(&spec_val) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(?e, subscription = %name, "bad subscription spec");
                break;
            }
        };
        let result = query::run(&root, &parsed);
        last_tick = ev.tick;

        if result.files.is_empty() && !result.is_fresh_instance {
            continue;
        }

        let mut pdu = IndexMap::new();
        pdu.insert(
            "version".into(),
            Value::String(crate::WATCHMAN_COMPAT_VERSION.into()),
        );
        pdu.insert("subscription".into(), Value::String(name.clone()));
        pdu.insert("clock".into(), Value::String(result.clock));
        pdu.insert("is_fresh_instance".into(), Value::Bool(false));
        pdu.insert("unilateral".into(), Value::Bool(true));
        pdu.insert(
            "root".into(),
            Value::String(root_path.to_string_lossy().into()),
        );
        pdu.insert("files".into(), Value::Array(result.files));
        if session.send(Value::Object(pdu)).is_err() {
            break;
        }
    }

    root.remove_subscription(&name);
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
) -> Result<(PathBuf, Arc<Root>, String, Value), CommandError> {
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
