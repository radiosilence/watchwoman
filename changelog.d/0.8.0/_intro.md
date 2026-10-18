This release is largely Nikita's work: trigger child processes are reaped, which stops a long-running daemon exhausting the process table, and jj worktrees get an event index instead of a full crawl.

### Added

- **jj worktrees get an event index instead of a full crawl.** A root containing `.jj` starts with an index of only the paths that change, discovering nested `.gitignore` files along those paths. Changes inside an ignored directory collapse into one rescan signal for that directory, so jj checks its tracked files there against its own working-copy state. A `.gitignore` edit, lost events or an exhausted index make the next query ask jj for a fresh scan. If another client issues a general query, subscription or trigger in the same root, the root is promoted to a full index for everyone. `status` reports which mode each root uses. Thanks to Nikita for the feature.

### Fixed

- **Trigger commands are reaped.** The daemon dropped each trigger's child process without waiting for it, so every run left a zombie behind. A long-running daemon with a busy trigger, such as jj's snapshot trigger, eventually exhausted the process table and starved the machine. Each trigger now waits for its command to exit before running again, which is also what watchman does. Thanks to Nikita for the fix.
- **`ignore_vcs` in `.watchmanconfig` is honoured.** The VCS directories that are reported but not recursed into were fixed at `.git`, `.hg` and `.svn`, so a `.jj` directory was indexed in full. The list now comes from the root's `ignore_vcs` when set, replacing the defaults as it does in watchman, and applies to the initial scan, recrawls and filesystem events alike. Thanks to Nikita for the fix.
- **`trigger-del` returns a boolean `deleted`**, as watchman does and the watchman client jj uses requires. Thanks to Nikita for the fix.
