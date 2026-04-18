//! Command dispatch table. Every wire-level command will eventually hang
//! off a `Handler` registered here. For now the module exists so the
//! daemon and tests can import a stable path.

use watchwoman_protocol::Value;

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("unknown command: {0}")]
    Unknown(String),
    #[error("bad args: {0}")]
    BadArgs(String),
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
}

pub type CommandResult = Result<Value, CommandError>;
