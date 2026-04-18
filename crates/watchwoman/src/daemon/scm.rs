//! SCM-aware clock resolution.
//!
//! Watchman's `scm:git:<mergebase>` / `scm:hg:<mergebase>` clocks mean
//! "files that would show up as changed if you ran `git diff` or
//! `hg status` between the given merge-base and the current working
//! copy."  We implement this by shelling out to the relevant VCS and
//! intersecting the set with the usual tick-based query filter.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::clock::ScmVcs;

/// Return the set of relative paths that have changed under `root`
/// since `mergebase`.  Returns `None` on any error — the caller treats
/// that as "fall back to a fresh query", matching watchman's
/// conservative behaviour.
pub fn changed_paths(root: &Path, vcs: ScmVcs, mergebase: &str) -> Option<HashSet<PathBuf>> {
    let out = match vcs {
        ScmVcs::Git => git_changed(root, mergebase),
        ScmVcs::Hg => hg_changed(root, mergebase),
    }?;
    let mut set = HashSet::with_capacity(out.len());
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        set.insert(PathBuf::from(line));
    }
    Some(set)
}

fn git_changed(root: &Path, mergebase: &str) -> Option<String> {
    // Resolve the merge-base against HEAD first so users can pass a
    // branch name or tag.  If mergebase is empty, use `HEAD~` — a
    // sensible default when the caller only wants "what's changed
    // recently".
    let target = if mergebase.is_empty() {
        "HEAD".to_owned()
    } else {
        run(root, "git", ["merge-base", "HEAD", mergebase])
            .map(|s| s.trim().to_owned())
            .unwrap_or_else(|| mergebase.to_owned())
    };
    // `--name-only` prints relative paths.  `--relative` keeps them
    // relative to the repo root even when run from a subdir, which
    // matches `<root>/<rel>` semantics in query results.
    let changed = run(
        root,
        "git",
        [
            "diff",
            "--name-only",
            "--relative",
            &format!("{target}..HEAD"),
        ],
    )?;
    // Also include staged + unstaged working-tree changes.
    let working =
        run(root, "git", ["diff", "--name-only", "--relative", "HEAD"]).unwrap_or_default();
    let untracked =
        run(root, "git", ["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
    Some(format!("{changed}\n{working}\n{untracked}"))
}

fn hg_changed(root: &Path, mergebase: &str) -> Option<String> {
    // Mercurial / Sapling: `hg status --rev` reports files changed
    // since the revision, and picks up working-copy edits.
    let rev_arg = if mergebase.is_empty() {
        ".^".to_owned()
    } else {
        mergebase.to_owned()
    };
    let out = run(root, "hg", ["status", "--no-status", "--rev", &rev_arg])?;
    Some(out)
}

fn run<I, S>(cwd: &Path, program: &str, args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        tracing::debug!(
            program,
            code = ?output.status.code(),
            "scm command failed"
        );
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}
