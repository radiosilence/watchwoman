### Added

- `-j` / `--json-command` — read a JSON PDU from stdin and send it to
  the daemon without going through clap's subcommand surface. Every
  tool in the watchman ecosystem that spawns `watchman -j` (git
  fsmonitor, Sapling, Metro, Jest when BSER isn't available) expects
  this. Pairs with `--no-pretty` for compact output.
- `-p` / `--persistent` — stay connected after the first response and
  stream unilateral PDUs until the daemon closes or SIGINT. Enables
  `watchman -j -p` for long-lived subscriptions.

### Changed

- `watchman` with no args prints help and exits 1 instead of clap's
  default error. Matches upstream behaviour.
