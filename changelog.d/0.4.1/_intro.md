### Fixed

- `install-agent` used `current_exe()` verbatim, which baked
  version-pinned paths like
  `~/.local/share/mise/installs/github-.../0.4.0/watchman` into the
  plist.  Next `mise upgrade` silently orphaned the agent.  The
  resolver now prefers (in order): the mise shim path, the brew
  prefix, `~/.cargo/bin`, and only falls back to `current_exe` with
  a warning if all stable locations are missing.  Re-run
  `watchwoman install-agent` to regenerate a plist with the new path.
