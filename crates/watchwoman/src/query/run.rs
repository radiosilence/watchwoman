//! Query executor that stitches the pieces together.

use std::sync::Arc;

use indexmap::IndexMap;
use watchwoman_protocol::Value;

use super::expr::{self, Expr};
use super::field::{self, Field};
use super::generator::Generators;
use crate::commands::CommandError;
use crate::daemon::clock::ClockSpec;
use crate::daemon::root::Root;

#[derive(Debug)]
pub struct QuerySpec {
    pub expression: Expr,
    pub fields: Vec<Field>,
    pub generators: Generators,
    pub since: Option<ClockSpec>,
    pub empty_on_fresh_instance: bool,
    pub always_include_directories: bool,
    pub dedup_results: bool,
    pub omit_changed_files: bool,
}

#[derive(Debug)]
pub struct QueryResult {
    pub files: Vec<Value>,
    pub clock: String,
    pub is_fresh_instance: bool,
}

pub fn parse_spec(raw: &Value) -> Result<QuerySpec, CommandError> {
    let map = raw
        .as_object()
        .ok_or_else(|| CommandError::BadArgs("query spec must be an object".into()))?
        .clone();

    let expression = match map.get("expression") {
        Some(v) => expr::parse(v)?,
        None => Expr::True,
    };
    let fields = match map.get("fields") {
        Some(v) => field::parse_list(v)?,
        None => field::default_fields(),
    };
    let generators = Generators::from_spec(&map)?;
    let since = map.get("since").and_then(|v| match v {
        Value::String(s) => Some(ClockSpec::parse(s)),
        _ => None,
    });
    Ok(QuerySpec {
        expression,
        fields,
        generators,
        since,
        empty_on_fresh_instance: map
            .get("empty_on_fresh_instance")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        always_include_directories: map
            .get("always_include_directories")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        dedup_results: map
            .get("dedup_results")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        omit_changed_files: map
            .get("omit_changed_files")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

pub fn run(root: &Arc<Root>, spec: &QuerySpec) -> QueryResult {
    let tree = root.tree.read();
    let mut files = Vec::new();
    let since_tick = spec.since.as_ref().map(|s| s.tick_against(&root.clock));
    let is_fresh_instance = since_tick.is_none();
    let include_dirs = spec.always_include_directories;

    if !(is_fresh_instance && spec.empty_on_fresh_instance) {
        for (rel, entry) in tree.iter() {
            if let Some(t) = since_tick {
                if entry.oclock <= t {
                    continue;
                }
            }
            if !include_dirs && entry.kind == crate::daemon::tree::FileKind::Dir {
                // watchman omits directories from query results unless
                // `always_include_directories` is set.
                continue;
            }
            if !spec.generators.accept(rel) {
                continue;
            }
            let ctx = expr::EvalCtx {
                clock: &root.clock,
                rel,
            };
            if expr::eval(&spec.expression, entry, &ctx) {
                files.push(field::render_row(
                    &root.path,
                    rel,
                    entry,
                    &spec.fields,
                    &root.clock,
                ));
            }
        }
    }

    QueryResult {
        files,
        clock: root.clock_string(),
        is_fresh_instance,
    }
}

pub fn result_to_pdu(root_path: &std::path::Path, result: QueryResult) -> Value {
    let mut m = IndexMap::new();
    m.insert(
        "version".to_owned(),
        Value::String(crate::WATCHMAN_COMPAT_VERSION.into()),
    );
    m.insert("clock".to_owned(), Value::String(result.clock));
    m.insert(
        "is_fresh_instance".to_owned(),
        Value::Bool(result.is_fresh_instance),
    );
    m.insert("files".to_owned(), Value::Array(result.files));
    m.insert(
        "root".to_owned(),
        Value::String(root_path.to_string_lossy().into()),
    );
    Value::Object(m)
}
