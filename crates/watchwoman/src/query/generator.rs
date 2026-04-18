//! Generators decide which files the evaluator iterates over.

use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};
use watchwoman_protocol::Value;

use crate::commands::CommandError;
use crate::daemon::clock::ClockSpec;

#[derive(Debug, Default)]
pub struct Generators {
    pub suffix: Option<Vec<String>>,
    pub glob: Option<Vec<GlobMatcher>>,
    pub paths: Option<Vec<PathSpec>>,
    pub since: Option<ClockSpec>,
}

#[derive(Debug, Clone)]
pub struct PathSpec {
    pub path: String,
    pub depth: Option<u32>,
}

impl Generators {
    pub fn from_spec(spec: &indexmap::IndexMap<String, Value>) -> Result<Self, CommandError> {
        let mut gen = Self::default();
        if let Some(v) = spec.get("suffix") {
            gen.suffix = Some(
                as_string_list(v)?
                    .into_iter()
                    .map(|s| s.to_ascii_lowercase())
                    .collect(),
            );
        }
        if let Some(v) = spec.get("glob") {
            let patterns = as_string_list(v)?;
            let mut matchers = Vec::with_capacity(patterns.len());
            for pat in patterns {
                let glob = GlobBuilder::new(&pat)
                    .literal_separator(true)
                    .build()
                    .map_err(|e| CommandError::BadArgs(format!("bad glob `{pat}`: {e}")))?
                    .compile_matcher();
                matchers.push(glob);
            }
            gen.glob = Some(matchers);
        }
        if let Some(v) = spec.get("path") {
            let arr = v
                .as_array()
                .ok_or_else(|| CommandError::BadArgs("`path` must be an array".into()))?;
            let mut specs = Vec::with_capacity(arr.len());
            for item in arr {
                match item {
                    Value::String(s) => specs.push(PathSpec {
                        path: s.clone(),
                        depth: None,
                    }),
                    Value::Object(o) => {
                        let path = o
                            .get("path")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                CommandError::BadArgs("path entry missing `path`".into())
                            })?
                            .to_owned();
                        let depth = o.get("depth").and_then(Value::as_i64).map(|d| d as u32);
                        specs.push(PathSpec { path, depth });
                    }
                    _ => {
                        return Err(CommandError::BadArgs(
                            "`path` entries must be strings or objects".into(),
                        ))
                    }
                }
            }
            gen.paths = Some(specs);
        }
        if let Some(v) = spec.get("since") {
            if let Some(s) = v.as_str() {
                gen.since = Some(ClockSpec::parse(s));
            }
        }
        Ok(gen)
    }

    pub fn accept(&self, rel: &Path) -> bool {
        if let Some(sufs) = &self.suffix {
            let ext = rel
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            if !matches!(&ext, Some(e) if sufs.iter().any(|s| s == e)) {
                return false;
            }
        }
        if let Some(globs) = &self.glob {
            if !globs.iter().any(|g| g.is_match(rel)) {
                return false;
            }
        }
        if let Some(paths) = &self.paths {
            if !paths.iter().any(|spec| accept_path(rel, spec)) {
                return false;
            }
        }
        true
    }
}

fn accept_path(rel: &Path, spec: &PathSpec) -> bool {
    let base = PathBuf::from(&spec.path);
    if spec.path.is_empty() {
        return spec
            .depth
            .is_none_or(|d| rel.components().count() as u32 <= d + 1);
    }
    let Ok(sub) = rel.strip_prefix(&base) else {
        return false;
    };
    match spec.depth {
        None => true,
        Some(d) => sub.components().count() as u32 <= d + 1,
    }
}

fn as_string_list(v: &Value) -> Result<Vec<String>, CommandError> {
    let arr = v
        .as_array()
        .ok_or_else(|| CommandError::BadArgs("expected an array of strings".into()))?;
    arr.iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| CommandError::BadArgs("expected a string".into()))
        })
        .collect()
}
