# Watchman wire protocol reference

Distilled from the upstream C++ source tree (cloned under
`reference/watchman/`) during initial scaffolding. File paths in this
document refer to that reference tree.

## PDU framing

Clients and the daemon negotiate one of three encodings on first byte.
The daemon sniffs magic:

- `0x00 0x01` → BSER v1
- `0x00 0x02` → BSER v2
- otherwise → JSON (newline-delimited, compact by default)

Reference: `watchman/PDU.cpp:76-87`, `watchman/PDU.cpp:89-146`.

### JSON

- Compact JSON per PDU, terminated by `\n`.
- `--no-pretty` disables multi-line pretty-printing in CLI mode.

### BSER v1

```
MAGIC(2)        = 0x00 0x01
LENGTH(varint)  = byte length of payload
PAYLOAD         = <Value>
```

### BSER v2

```
MAGIC(2)        = 0x00 0x02
CAPS(u32 LE)    = per-message capability bits
LENGTH(varint)  = byte length of payload
PAYLOAD         = <Value>
```

Capability bits (`watchman/bser.cpp:352-398`):

- `0x1` `BSER_CAP_DISABLE_UNICODE` — emit all strings as BYTESTRING.
- `0x2` `BSER_CAP_DISABLE_UNICODE_FOR_ERRORS` — errors as BYTESTRING.

### BSER type tags

| tag  | meaning      |
|------|--------------|
| 0x00 | ARRAY        |
| 0x01 | OBJECT       |
| 0x02 | BYTESTRING   |
| 0x03 | INT8         |
| 0x04 | INT16        |
| 0x05 | INT32        |
| 0x06 | INT64        |
| 0x07 | REAL (f64)   |
| 0x08 | TRUE         |
| 0x09 | FALSE        |
| 0x0a | NULL         |
| 0x0b | TEMPLATE     |
| 0x0c | SKIP         |
| 0x0d | UTF8STRING   |

- Integers are tagged with the smallest fitting width; all little-endian.
- Strings: `[tag][varint len][bytes...]`.
- Arrays/Objects: `[tag][varint count][items...]`; objects alternate
  key-as-string then value.
- Templates: `[0x0b][KEYARRAY][varint rowcount][values]` — used for
  arrays of objects sharing key layout. `SKIP` marks an absent field.

Reference: `watchman/bser.cpp:31-240`, `watchman/python/pywatchman/pybser.py`.

## Socket path

Precedence:

1. `--sockname <path>`
2. `$WATCHMAN_SOCK`
3. Platform default. On recent builds: `$XDG_STATE_HOME/watchman/<user>-state/sock`
   on Linux, or `~/.local/state/watchman/<user>-state/sock` on macOS.
4. Legacy: `$TMPDIR/<user>-state/sock` or `/tmp/<user>-state/sock`.

Reference: `watchman/sockname.{h,cpp}`, `watchman/Options.h:16-38`.

## Commands

Every command is a JSON array: `[ "name", ...args ]`. Response is a JSON
object. Error responses include an `"error"` key and no payload keys.

### Server info

- `get-sockname` — `{ "sockname", "unix_domain", "version" }`
- `get-pid` — `{ "pid" }`
- `version [{ required?: [cap], optional?: [cap] }]` —
  `{ "version", "buildinfo", "capabilities": { cap: bool }, "error"? }`
- `list-capabilities` — `{ "capabilities": [name, ...] }`
- `get-config <root>` — `{ "config": <object> }`
- `log-level [level]`, `log <msg>` — logging controls
- `shutdown-server` — `{ "shutdown": true }` then closes.

### Watch lifecycle

- `watch <path>` — `{ "watch", "watcher", "warning"? }`
- `watch-project <path>` — `{ "watch", "relative_path"?, "watcher" }`
- `watch-list` — `{ "roots": [path] }`
- `watch-del <path>` — `{ "watch-del": true, "root" }`
- `watch-del-all` — `{ "roots": [path] }`

### Queries

- `query <root> <spec>` — structured query (see below).
- `find <root> <pattern...>` — legacy glob finder.
- `since <root> <clock>` — legacy delta.

### Subscriptions

- `subscribe <root> <name> <spec>` — initial response `{ "subscribe",
  "clock", "files", "is_fresh_instance", "root" }`; subsequent
  unilateral PDUs carry `{ "subscription", "files", "clock",
  "unilateral": true, "root", "is_fresh_instance" }`.
- `unsubscribe <root> <name>` — `{ "unsubscribed": true }`.
- `flush-subscriptions <root> [timeout_ms]` — `{ "synced": [name] }`.

Query-spec options specific to subscriptions: `defer`, `drop`,
`defer_vcs`, `settle_period`, `settle_timeout`.

### State assertions

- `state-enter <root> <name | { name, metadata?, sync_timeout? }>` —
  `{ "state-enter": name }`.
- `state-leave <root> <name | { name, metadata? }>` —
  `{ "state-leave": name }`.

### Triggers

- `trigger <root> <spec>` — installs a side-effect trigger.
- `trigger-list <root>` — `{ "triggers": [...] }`.
- `trigger-del <root> <name>` — `{ "deleted": 0 | 1 }`.

## Query language

### Query-spec keys

- `expression` — expression tree (see below).
- `fields` — array of field names to return per result row.
- `since` — clock or cursor.
- `generator`:
  - `glob` — `[pattern, ...]`
  - `suffix` — `[ext, ...]`
  - `path` — `[ "dir" | { path, depth }, ...]`
  - implicit: if none specified, walk all files.
- `relative_root`, `case_sensitive`, `dedup_results`, `sync_timeout`,
  `lock_timeout`, `settle_period`, `settle_timeout`,
  `empty_on_fresh_instance`, `omit_changed_files`,
  `always_include_directories`, `fail_if_no_saved_state`.

### Expression operators

Logical: `allof`, `anyof`, `not`, `true`, `false`.

String: `name`, `iname`, `match`, `imatch`, `pcre`, `ipcre`,
`suffix`, `dirname`, `idirname`.

File props: `type` (`f`/`d`/`l`/`b`/`c`/`p`/`s`), `size`
(with `"eq"|"ne"|"lt"|"le"|"gt"|"ge"`), `exists`, `empty`, `since`.

Reference: `watchman/query/TermRegistry.{h,cpp}` and sibling
operator impls.

### Fields

`name`, `exists`, `size`, `mode`, `uid`, `gid`, `ino`, `dev`, `nlink`,
`mtime`, `mtime_ms`, `mtime_ns`, `mtime_f`, `ctime` (and the same
variants), `type`, `new`, `cclock`, `oclock`, `symlink_target`,
`content.sha1hex`.

Reference: `watchman/query/fieldlist.cpp`.

### Clock specs

Opaque strings from the daemon's perspective:

- `c:<root_number>:<ticks>:<start_time>:<pid>` — normal clock.
- `<integer>` — bare tick count.
- `n:<cursor_name>` — named cursor, auto-managed by the daemon.
- `scm:<vcs>:<merge-base>` — SCM-aware window.

## Capabilities advertised

Commands (`command-<name>`), expression terms (`term-<name>`),
generators (`glob_generator`, `suffix-set`, `path_generator`),
plus: `wildmatch`, `wildmatch-multislash`, `relative_root`,
`dedup_results`, `clock-sync-timeout`, `scm-hg`, `scm-git`,
`scm-since`, `saved-state-local`, `bser-v2`.

Reference: `watchman/CommandRegistry.cpp:106-126`,
`watchman/query/TermRegistry.cpp:20-35`.
