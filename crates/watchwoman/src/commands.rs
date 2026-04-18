//! Command dispatch.  Every watchman command lands in [`dispatch`],
//! which peels the command name off the PDU, looks up the handler, and
//! formats a JSON response object.

use std::sync::Arc;

use indexmap::IndexMap;
use watchwoman_protocol::Value;

use crate::daemon::session::Session;
use crate::daemon::state::DaemonState;

pub mod clock;
pub mod debug;
pub mod info;
pub mod query;
pub mod state;
pub mod subscribe;
pub mod trigger;
pub mod watch;

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("unknown command: {0}")]
    Unknown(String),
    #[error("bad args: {0}")]
    BadArgs(String),
    #[error("no such root: {0}")]
    UnknownRoot(String),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
    #[error("internal: {0:#}")]
    Internal(#[from] anyhow::Error),
}

pub type CommandResult = Result<Value, CommandError>;

/// Entry point invoked for every PDU the server reads.
pub fn dispatch(state: &Arc<DaemonState>, session: &Session, pdu: Value) -> Value {
    match dispatch_inner(state, session, pdu) {
        Ok(v) => v,
        Err(e) => {
            let mut m = IndexMap::new();
            m.insert("error".to_owned(), Value::String(format!("{e:#}")));
            Value::Object(m)
        }
    }
}

fn dispatch_inner(state: &Arc<DaemonState>, session: &Session, pdu: Value) -> CommandResult {
    let arr = pdu
        .as_array()
        .ok_or_else(|| CommandError::BadArgs("PDU must be an array".into()))?;
    let name = arr
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("missing command name".into()))?
        .to_owned();
    let args = &arr[1..];

    match name.as_str() {
        "get-sockname" => info::get_sockname(state),
        "get-pid" => info::get_pid(),
        "version" => info::version(args),
        "list-capabilities" => info::list_capabilities(),
        "get-config" => info::get_config(args),
        "log-level" => info::log_level(args),
        "log" => info::log(args),
        "watch" => watch::watch(state, args),
        "watch-project" => watch::watch_project(state, args),
        "watch-list" => watch::watch_list(state),
        "watch-del" => watch::watch_del(state, args),
        "watch-del-all" => watch::watch_del_all(state),
        "clock" => clock::clock(state, args),
        "query" => query::query(state, args),
        "find" => query::find(state, args),
        "since" => query::since(state, args),
        "subscribe" => subscribe::subscribe(state, session, args),
        "unsubscribe" => subscribe::unsubscribe(state, args),
        "flush-subscriptions" => subscribe::flush_subscriptions(state, args),
        "state-enter" => state::state_enter(state, args),
        "state-leave" => state::state_leave(state, args),
        "trigger" => trigger::trigger(state, args),
        "trigger-list" => trigger::trigger_list(state, args),
        "trigger-del" => trigger::trigger_del(state, args),
        "debug-recrawl" => debug::recrawl(state, args),
        "debug-ageout" => debug::ageout(state, args),
        "debug-show-cursors" => debug::show_cursors(state, args),
        "debug-poll-for-settle" => Ok(obj([("collected", Value::Bool(true))])),
        "shutdown-server" => {
            state.request_shutdown();
            let mut m = IndexMap::new();
            m.insert("shutdown".to_owned(), Value::Bool(true));
            Ok(Value::Object(m))
        }
        other => Err(CommandError::Unknown(other.to_owned())),
    }
}

/// Helper: build a JSON object from `[(key, value)]` pairs in source order.
pub(crate) fn obj<const N: usize>(entries: [(&str, Value); N]) -> Value {
    let mut m = IndexMap::with_capacity(N);
    for (k, v) in entries {
        m.insert(k.to_owned(), v);
    }
    Value::Object(m)
}
