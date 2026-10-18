### Fixed

- Orphan-socket wedge: CLI auto-spawn only triggered when the socket
  file was missing. After a `shutdown-server` or a daemon crash, the
  socket file could linger with nobody listening, and every
  subsequent call errored with "Connection refused" until someone
  manually removed it. Now we attempt to connect first; on
  `ConnectionRefused`, we clean the orphan and respawn.

### Added

- Durable triggers. `trigger` / `trigger-del` now persist to
  `<state-dir>/roots/<root-slug>/triggers.json` on write, and
  `register_root` rehydrates them on daemon start, re-spawning each
  trigger's tick loop. Daemon restarts are invisible to tools that
  installed triggers earlier.
- `watchman-diag` companion binary — dumps version, capabilities,
  sockname, pid, watched roots, per-root cursors / config / triggers
  as one JSON document. Pipe into a gist when opening an issue.
- `watchmanctl` companion binary — thin control wrapper:
  `watchmanctl status | shutdown | log-level [level] | recrawl`.
- CLI subcommands: `debug-recrawl`, `debug-ageout`, `debug-show-cursors`,
  `debug-poll-for-settle` (were accessible only via `-j` before).
