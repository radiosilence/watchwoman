Parity polish: every CLI flag and command in upstream watchman's
help output is now accepted by watchwoman.  47 commands and ~15
flags audited against `watchman --help` from 2026.03.30.00.

### Added

- CLI flag aliases so tools that hard-code the upstream spelling work
  without adjustment: `-U`/`-u`/`--unix-listener-path` alias for
  `--sockname`, `--foreground` alias for the internal
  `--foreground-daemon`.
- `-o`/`--logfile PATH` — redirects tracing output to the given file
  (append mode); `-` restores stderr.
- `--log-level 0|1|2|3` — maps to `off`/`warn`/`debug`/`trace`.
- `--pidfile PATH` — written on foreground-daemon start, for init
  scripts and process monitors that grep it.
- CLI subcommands added to match `watchman --help`: `find`, `since`,
  `trigger`, `trigger-list`, `trigger-del`, `get-log`,
  `global-log-level`, `debug-status`, `debug-root-status`,
  `debug-watcher-info`, `debug-watcher-info-clear`,
  `debug-get-asserted-states`, `debug-get-subscriptions`,
  `debug-contenthash`, `debug-kqueue-and-fsevents-recrawl`,
  `debug-fsevents-inject-drop`, `debug-set-parallel-crawl`,
  `debug-set-subscriptions-paused`, `debug-symlink-target-cache`.

### Removed

- Accept-and-ignore-only CLI compat flags that no real tool in the
  ecosystem actually passes: `--inetd`, `-S`/`--no-site-spawner`,
  `-n`/`--no-save-state`, `--statefile`, `--no-local`,
  `--named-pipe-path`, `--pretty`.  Parity is carried by the flags
  that do something; the rest were cargo-cult.

### Verified

End-to-end tested 49 subcommand + flag scenarios against a live
daemon; each returns a shape-compatible JSON object.  Parity with
watchman 2026.03.30.00 for everything a real tool uses.
