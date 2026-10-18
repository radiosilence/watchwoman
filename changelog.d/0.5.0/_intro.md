### Added

- `watchman status` — human-readable daemon report: uptime, RSS, CPU,
  per-root file counts, idle time, subscription/trigger counts, health
  (`active` / `idle` / `stale` / `dead`), and the last 64 garbage-
  collected watches.  `--json` for scripting; the server response
  always is JSON and the CLI only formats it.  Added
  `cmd-status` to `list-capabilities`.
- Watch garbage collector.  A background sweep, zero-conf, runs every
  60 s and reaps:
    - **dead** roots — directory missing from disk on two consecutive
      ticks (~60-120 s grace); usually a git worktree that was
      removed or a volume that unmounted.
    - **stale** roots — no subscriptions, no installed triggers, and no
      commands for 14 days.  Active roots are never stale-reaped
      regardless of idle time.

  Reaps log at `WARN` so the long-running LaunchAgent can't silently
  throw away a watch you cared about, and they surface in
  `watchman status` for post-mortem.
