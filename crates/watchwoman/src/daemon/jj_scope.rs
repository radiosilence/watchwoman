use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// Cache only the ignore files along paths that actually receive events.
/// A cold jj watch never walks the working copy to discover nested rules.
#[derive(Default)]
pub struct JjScope {
    rules: HashMap<PathBuf, Option<Gitignore>>,
}

impl JjScope {
    pub fn clear(&mut self) {
        self.rules.clear();
    }

    /// The outermost ignored directory on this path. A synthetic
    /// `<dir>/.gitignore` event makes jj rescan the prefix with its own
    /// tracked-file state, including tracked exceptions to ignore rules.
    pub fn ignored_directory(
        &mut self,
        root: &Path,
        rel: &Path,
        target_is_dir: bool,
    ) -> Option<PathBuf> {
        let mut ancestors = vec![root.to_path_buf()];
        let mut prefix = PathBuf::new();
        let components: Vec<_> = rel.components().collect();
        for (index, component) in components.iter().enumerate() {
            prefix.push(component.as_os_str());
            let is_dir = index + 1 < components.len() || target_is_dir;
            if !is_dir {
                break;
            }
            let abs = root.join(&prefix);
            let mut ignored = false;
            for ancestor in &ancestors {
                if let Some(matcher) = self.matcher(ancestor) {
                    let result = matcher.matched(&abs, true);
                    if !result.is_none() {
                        ignored = result.is_ignore();
                    }
                }
            }
            if ignored {
                return Some(prefix);
            }
            ancestors.push(abs);
        }
        None
    }

    fn matcher(&mut self, dir: &Path) -> Option<&Gitignore> {
        if self.rules.len() >= 8192 && !self.rules.contains_key(dir) {
            self.rules.clear();
        }
        self.rules
            .entry(dir.to_path_buf())
            .or_insert_with(|| {
                let path = dir.join(".gitignore");
                if !path.is_file() {
                    return None;
                }
                let mut builder = GitignoreBuilder::new(dir);
                if let Some(err) = builder.add(&path) {
                    tracing::warn!(?err, path = %path.display(), "partially invalid .gitignore");
                }
                match builder.build() {
                    Ok(matcher) => Some(matcher),
                    Err(err) => {
                        tracing::warn!(?err, path = %path.display(), "failed to compile .gitignore");
                        None
                    }
                }
            })
            .as_ref()
    }
}
