//! End-to-end check that `list-capabilities` advertises every command
//! the dispatcher actually handles plus the platform watcher backend.
//! Clients doing `required` capability probes used to spuriously miss
//! against real handlers — if this drifts again, catch it here.

use indexmap::IndexMap;
use watchwoman_protocol::Value;
use watchwoman_tests::Harness;

fn obj(entries: &[(&str, Value)]) -> Value {
    let mut m = IndexMap::with_capacity(entries.len());
    for (k, v) in entries {
        m.insert((*k).to_owned(), v.clone());
    }
    Value::Object(m)
}

/// Capabilities that were previously implemented but not advertised.
/// Kept in source form (rather than pulled from the constant) so a
/// regression in the advertised list shows up as a diff on this test,
/// not a silent recompile.
const EXPECTED_CAPS: &[&str] = &[
    "cmd-debug-contenthash",
    "cmd-debug-fsevents-inject-drop",
    "cmd-debug-get-asserted-states",
    "cmd-debug-get-subscriptions",
    "cmd-debug-kqueue-and-fsevents-recrawl",
    "cmd-debug-root-status",
    "cmd-debug-set-parallel-crawl",
    "cmd-debug-set-subscriptions-paused",
    "cmd-debug-status",
    "cmd-debug-symlink-target-cache",
    "cmd-debug-watcher-info",
    "cmd-debug-watcher-info-clear",
    "cmd-get-log",
    "cmd-global-log-level",
    "field-content.sha1hex",
];

/// Commands we deliberately refuse — must NOT be advertised.
const REFUSED_CAPS: &[&str] = &["cmd-debug-drop-privs", "cmd-debug-poison"];

#[test]
fn list_capabilities_advertises_every_handled_command() {
    let h = Harness::spawn().expect("spawn daemon");
    let mut c = h.client().unwrap();

    let resp = c.call("list-capabilities", []).unwrap();
    let caps = resp
        .as_object()
        .and_then(|o| o.get("capabilities"))
        .and_then(Value::as_array)
        .expect("capabilities array");
    let names: Vec<&str> = caps.iter().filter_map(Value::as_str).collect();

    for want in EXPECTED_CAPS {
        assert!(
            names.contains(want),
            "expected `{want}` in list-capabilities; got {names:?}"
        );
    }
    for refused in REFUSED_CAPS {
        assert!(
            !names.contains(refused),
            "`{refused}` must not be advertised — it's refused by the dispatcher"
        );
    }

    // Platform-appropriate watcher backend.
    #[cfg(target_os = "macos")]
    let want_watcher = "watcher-fsevents";
    #[cfg(target_os = "linux")]
    let want_watcher = "watcher-inotify";
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let want_watcher = "watcher-kqueue";
    assert!(
        names.contains(&want_watcher),
        "expected `{want_watcher}` in list-capabilities; got {names:?}"
    );
}

#[test]
fn version_required_probe_accepts_newly_advertised_caps() {
    let h = Harness::spawn().expect("spawn daemon");
    let mut c = h.client().unwrap();

    let required: Vec<Value> = EXPECTED_CAPS
        .iter()
        .map(|s| Value::String((*s).into()))
        .collect();
    let spec = obj(&[("required", Value::Array(required))]);
    let resp = c.call("version", [spec]).unwrap();

    let map = resp.as_object().expect("object response");
    assert!(
        map.get("error").is_none(),
        "version probe returned error for required caps: {:?}",
        map.get("error")
    );
    let caps = map
        .get("capabilities")
        .and_then(Value::as_object)
        .expect("capabilities sub-object");
    for want in EXPECTED_CAPS {
        let have = caps.get(*want).and_then(Value::as_bool).unwrap_or(false);
        assert!(have, "version reported `{want}` as missing");
    }
}
