Parity push. Every item in `docs/PARITY.md` either ticks now or has
an explicit "intentionally out of scope" note.

### Added

- **Expression terms** — `pcre` and `ipcre` (regex-backed; `regex`
  crate). Capability list now advertises them.
- **Query spec** — `relative_root` narrows the query to a subdir of
  the watched root and strips that prefix from result names;
  `case_sensitive` flag affects `name`/`iname`/`dirname` eval;
  `dedup_results` accepted on spec; `uid`/`gid` fields now populated
  from stat.
- **Named cursors** — `since: "n:my-cursor"` looks up a per-root tick
  cursor and advances it atomically with each query (classic watchman
  semantics).
- **SCM-aware clocks** — `scm:git:<mergebase>` and `scm:hg:<mergebase>`
  (also `scm:sapling:` / `scm:sl:`) shell out to the relevant VCS and
  filter the query to files changed since the merge-base plus any
  uncommitted/untracked changes. Capabilities: `scm-git`, `scm-hg`,
  `scm-since`.
- **Debug commands** — `debug-recrawl` forces a full rescan,
  `debug-ageout` and `debug-poll-for-settle` return clean shapes for
  tools that probe them, `debug-show-cursors` dumps named cursors.
- **Companion binaries** — `watchman-wait` (blocks on matching changes)
  and `watchman-make` (debounced command runner). Both shipped as
  standalone binaries in the release tarballs and via `brew install`.

### Notes

- `sync_timeout`, `lock_timeout`, `settle_period`, `settle_timeout` on
  query specs are accepted but still always settle immediately — our
  event batching runs at 5 ms, so the watchman semantics of "wait
  until the kernel drains" is effectively always true.
