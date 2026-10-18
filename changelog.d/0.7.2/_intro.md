### Fixed

- **The CLI honours `--output-encoding=bser-v2` (and `bser`/`bser2`).** Watchman clients such as jj discover the daemon socket by running `get-sockname` with `--output-encoding=bser-v2`; the CLI rejected that spelling and the other BSER values still printed JSON, so discovery failed. `bser-v2` is accepted as an alias and stdout now uses the protocol codec for commands, stdin PDUs, errors, status and persistent responses. Thanks to Nikita for the fix.
