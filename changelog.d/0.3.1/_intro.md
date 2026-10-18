### Added

- `watchwoman install-agent` / `uninstall-agent` on macOS — drops a
  LaunchAgent plist at `~/Library/LaunchAgents/cc.blit.watchwoman.plist`
  and bootstraps it with `launchctl`, so the daemon auto-starts at
  login and restarts on crash.  Not run automatically; opt in.
