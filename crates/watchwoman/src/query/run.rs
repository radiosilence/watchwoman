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
    pub relative_root: Option<std::path::PathBuf>,
    pub empty_on_fresh_instance: bool,
    pub always_include_directories: bool,
    pub dedup_results: bool,
    pub omit_changed_files: bool,
    pub case_sensitive: bool,
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
    let relative_root = map
        .get("relative_root")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from);
    Ok(QuerySpec {
        expression,
        fields,
        generators,
        since,
        relative_root,
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
        case_sensitive: map
            .get("case_sensitive")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

pub fn run(root: &Arc<Root>, spec: &QuerySpec) -> QueryResult {
    // Named-cursor resolution.  Captured before we read the tree so
    // the cursor advances atomically with the query.
    let (cursor_since, cursor_name) = resolve_named_cursor(root, spec);
    let scm_allowed = resolve_scm_set(root, spec);

    let tree = root.tree.read();
    let mut files = Vec::new();
    let since_tick =
        cursor_since.or_else(|| spec.since.as_ref().map(|s| s.tick_against(&root.clock)));
    let is_fresh_instance = since_tick.is_none();
    let include_dirs = spec.always_include_directories;

    let rel_root = spec.relative_root.as_deref();

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
            // relative_root narrows the query to a subdir of the
            // watched root. Paths in results are also stripped of that
            // prefix so callers receive root-relative names.
            let display_rel = if let Some(base) = rel_root {
                match rel.strip_prefix(base) {
                    Ok(p) => p,
                    Err(_) => continue,
                }
            } else {
                rel
            };
            if !spec.generators.accept(display_rel) {
                continue;
            }
            if let Some(allowed) = &scm_allowed {
                if !allowed.contains(rel) && !allowed.contains(display_rel) {
                    continue;
                }
            }
            let ctx = expr::EvalCtx {
                clock: &root.clock,
                rel: display_rel,
                case_sensitive: spec.case_sensitive,
            };
            if expr::eval(&spec.expression, entry, &ctx) {
                files.push(field::render_row(
                    &root.path,
                    display_rel,
                    entry,
                    &spec.fields,
                    &root.clock,
                ));
            }
        }
    }

    // After a query resolves, any named cursor advances to the tick
    // we just observed so the next query only sees newer changes.
    if let Some(name) = cursor_name {
        root.set_cursor(&name, root.clock.current_tick());
    }

    QueryResult {
        files,
        clock: root.clock_string(),
        is_fresh_instance,
    }
}

fn resolve_named_cursor(root: &Arc<Root>, spec: &QuerySpec) -> (Option<u64>, Option<String>) {
    if let Some(ClockSpec::Named(name)) = &spec.since {
        let tick = root.cursor_tick(name);
        (Some(tick), Some(name.clone()))
    } else {
        (None, None)
    }
}

fn resolve_scm_set(
    root: &Arc<Root>,
    spec: &QuerySpec,
) -> Option<std::collections::HashSet<std::path::PathBuf>> {
    match &spec.since {
        Some(ClockSpec::Scm { vcs, mergebase }) => {
            crate::daemon::scm::changed_paths(&root.path, *vcs, mergebase)
        }
        _ => None,
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
