//! Query expression tree.
//!
//! Parser is lenient in the spirit of watchman: unknown operators don't
//! crash the daemon, they become a `False` terminal so the query returns
//! nothing rather than returning everything. That keeps CI systems from
//! accidentally shipping broken queries that match the world.

use std::path::Path;

use globset::GlobMatcher;
use watchwoman_protocol::Value;

use crate::commands::CommandError;
use crate::daemon::clock::ClockSpec;
use crate::daemon::tree::{FileEntry, FileKind};

#[derive(Debug)]
pub enum Expr {
    True,
    False,
    AllOf(Vec<Expr>),
    AnyOf(Vec<Expr>),
    Not(Box<Expr>),
    Name {
        names: Vec<String>,
        scope: MatchScope,
        case_sensitive: bool,
    },
    Match {
        matcher: GlobMatcher,
        scope: MatchScope,
    },
    Suffix(Vec<String>),
    Type(FileKind),
    Size(CmpOp, u64),
    Exists,
    Empty,
    Since(ClockSpec),
    Dirname {
        path: String,
        case_sensitive: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchScope {
    Basename,
    Wholename,
}

#[derive(Debug, Clone, Copy)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl CmpOp {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "eq" => Self::Eq,
            "ne" => Self::Ne,
            "lt" => Self::Lt,
            "le" => Self::Le,
            "gt" => Self::Gt,
            "ge" => Self::Ge,
            _ => return None,
        })
    }
    pub fn apply(self, lhs: u64, rhs: u64) -> bool {
        match self {
            Self::Eq => lhs == rhs,
            Self::Ne => lhs != rhs,
            Self::Lt => lhs < rhs,
            Self::Le => lhs <= rhs,
            Self::Gt => lhs > rhs,
            Self::Ge => lhs >= rhs,
        }
    }
}

pub fn parse(value: &Value) -> Result<Expr, CommandError> {
    match value {
        Value::String(s) => match s.as_str() {
            "true" => Ok(Expr::True),
            "false" => Ok(Expr::False),
            "exists" => Ok(Expr::Exists),
            "empty" => Ok(Expr::Empty),
            other => Err(CommandError::BadArgs(format!(
                "unknown bare expression `{other}`"
            ))),
        },
        Value::Array(items) => parse_array(items),
        other => Err(CommandError::BadArgs(format!(
            "expected expression array, got {other:?}"
        ))),
    }
}

fn parse_array(items: &[Value]) -> Result<Expr, CommandError> {
    let head = items
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| CommandError::BadArgs("empty expression".into()))?;
    let rest = &items[1..];
    Ok(match head {
        "true" => Expr::True,
        "false" => Expr::False,
        "exists" => Expr::Exists,
        "empty" => Expr::Empty,
        "not" => {
            let inner = rest
                .first()
                .ok_or_else(|| CommandError::BadArgs("`not` requires an operand".into()))?;
            Expr::Not(Box::new(parse(inner)?))
        }
        "allof" => Expr::AllOf(rest.iter().map(parse).collect::<Result<_, _>>()?),
        "anyof" => Expr::AnyOf(rest.iter().map(parse).collect::<Result<_, _>>()?),
        "name" | "iname" => {
            let case_sensitive = head == "name";
            let (names, scope) = parse_name_args(rest)?;
            Expr::Name {
                names,
                scope,
                case_sensitive,
            }
        }
        "match" | "imatch" => {
            let case_insensitive = head == "imatch";
            let pattern = rest
                .first()
                .and_then(Value::as_str)
                .ok_or_else(|| CommandError::BadArgs("`match` requires a pattern".into()))?;
            let scope = rest
                .get(1)
                .and_then(Value::as_str)
                .map(parse_scope)
                .transpose()?
                .unwrap_or(MatchScope::Basename);
            let mut builder = globset::GlobBuilder::new(pattern);
            builder.case_insensitive(case_insensitive);
            builder.literal_separator(matches!(scope, MatchScope::Wholename));
            let matcher = builder
                .build()
                .map_err(|e| CommandError::BadArgs(format!("bad match pattern: {e}")))?
                .compile_matcher();
            Expr::Match { matcher, scope }
        }
        "suffix" => {
            let list = flatten_strings(rest)?;
            Expr::Suffix(list.into_iter().map(|s| s.to_ascii_lowercase()).collect())
        }
        "type" => {
            let t = rest
                .first()
                .and_then(Value::as_str)
                .ok_or_else(|| CommandError::BadArgs("`type` requires a kind".into()))?;
            Expr::Type(match t {
                "f" => FileKind::File,
                "d" => FileKind::Dir,
                "l" => FileKind::Symlink,
                "b" => FileKind::Block,
                "c" => FileKind::Char,
                "p" => FileKind::Fifo,
                "s" => FileKind::Socket,
                other => {
                    return Err(CommandError::BadArgs(format!("unknown type `{other}`")));
                }
            })
        }
        "size" => {
            let op = rest
                .first()
                .and_then(Value::as_str)
                .and_then(CmpOp::parse)
                .ok_or_else(|| CommandError::BadArgs("`size` requires a comparison op".into()))?;
            let bytes = rest
                .get(1)
                .and_then(Value::as_i64)
                .ok_or_else(|| CommandError::BadArgs("`size` requires bytes".into()))?;
            Expr::Size(op, bytes as u64)
        }
        "since" => {
            let s = rest
                .first()
                .and_then(Value::as_str)
                .ok_or_else(|| CommandError::BadArgs("`since` requires a clock".into()))?;
            Expr::Since(ClockSpec::parse(s))
        }
        "dirname" | "idirname" => {
            let case_sensitive = head == "dirname";
            let path = rest
                .first()
                .and_then(Value::as_str)
                .ok_or_else(|| CommandError::BadArgs("`dirname` requires a path".into()))?
                .to_owned();
            Expr::Dirname {
                path,
                case_sensitive,
            }
        }
        other => {
            return Err(CommandError::BadArgs(format!(
                "unknown expression operator `{other}`"
            )))
        }
    })
}

fn parse_scope(s: &str) -> Result<MatchScope, CommandError> {
    match s {
        "basename" => Ok(MatchScope::Basename),
        "wholename" => Ok(MatchScope::Wholename),
        other => Err(CommandError::BadArgs(format!(
            "unknown scope `{other}` (use basename or wholename)"
        ))),
    }
}

fn parse_name_args(rest: &[Value]) -> Result<(Vec<String>, MatchScope), CommandError> {
    let first = rest
        .first()
        .ok_or_else(|| CommandError::BadArgs("`name` requires at least a name".into()))?;
    let names = flatten_strings(std::slice::from_ref(first))?;
    let scope = rest
        .get(1)
        .and_then(Value::as_str)
        .map(parse_scope)
        .transpose()?
        .unwrap_or(MatchScope::Basename);
    Ok((names, scope))
}

fn flatten_strings(items: &[Value]) -> Result<Vec<String>, CommandError> {
    let mut out = Vec::new();
    for v in items {
        match v {
            Value::String(s) => out.push(s.clone()),
            Value::Array(inner) => {
                for item in inner {
                    if let Some(s) = item.as_str() {
                        out.push(s.to_owned());
                    }
                }
            }
            _ => return Err(CommandError::BadArgs("expected string or array".into())),
        }
    }
    Ok(out)
}

/// Holds context the evaluator needs beyond the entry itself.
pub struct EvalCtx<'a> {
    pub clock: &'a crate::daemon::clock::Clock,
    pub rel: &'a Path,
}

pub fn eval(expr: &Expr, entry: &FileEntry, ctx: &EvalCtx<'_>) -> bool {
    match expr {
        Expr::True => true,
        Expr::False => false,
        Expr::AllOf(list) => list.iter().all(|e| eval(e, entry, ctx)),
        Expr::AnyOf(list) => list.iter().any(|e| eval(e, entry, ctx)),
        Expr::Not(inner) => !eval(inner, entry, ctx),
        Expr::Exists => entry.exists,
        Expr::Empty => entry.size == 0,
        Expr::Name {
            names,
            scope,
            case_sensitive,
        } => match_name(ctx.rel, names, *scope, *case_sensitive),
        Expr::Match { matcher, scope } => match scope {
            MatchScope::Basename => ctx
                .rel
                .file_name()
                .is_some_and(|b| matcher.is_match(Path::new(b))),
            MatchScope::Wholename => matcher.is_match(ctx.rel),
        },
        Expr::Suffix(exts) => ctx
            .rel
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .is_some_and(|ext| exts.iter().any(|s| s == &ext)),
        Expr::Type(kind) => &entry.kind == kind,
        Expr::Size(op, rhs) => op.apply(entry.size, *rhs),
        Expr::Since(spec) => entry.oclock > spec.tick_against(ctx.clock),
        Expr::Dirname {
            path,
            case_sensitive,
        } => match_dirname(ctx.rel, path, *case_sensitive),
    }
}

fn match_name(rel: &Path, names: &[String], scope: MatchScope, case_sensitive: bool) -> bool {
    let hay: String = match scope {
        MatchScope::Basename => rel
            .file_name()
            .map(|b| b.to_string_lossy().into_owned())
            .unwrap_or_default(),
        MatchScope::Wholename => rel.to_string_lossy().to_string(),
    };
    if case_sensitive {
        names.iter().any(|n| n == &hay)
    } else {
        names.iter().any(|n| n.eq_ignore_ascii_case(&hay))
    }
}

fn match_dirname(rel: &Path, target: &str, case_sensitive: bool) -> bool {
    let parent = rel.parent().map(|p| p.to_string_lossy().into_owned());
    match parent {
        None => false,
        Some(p) => {
            if case_sensitive {
                p == target || p.starts_with(&format!("{target}/"))
            } else {
                p.eq_ignore_ascii_case(target)
                    || p.to_ascii_lowercase()
                        .starts_with(&format!("{}/", target.to_ascii_lowercase()))
            }
        }
    }
}
