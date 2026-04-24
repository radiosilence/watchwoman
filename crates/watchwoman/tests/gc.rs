use watchwoman_protocol::Value;
use watchwoman_tests::{Harness, Scratch};

/// A watched root whose directory disappears from disk should be
/// reaped by the GC on its second sweep.  We skip the real 60 s
/// timer by sending the hidden `debug-gc-tick` command.
#[test]
fn dead_root_is_reaped() {
    let h = Harness::spawn().expect("spawn daemon");
    let scratch = Scratch::new().unwrap();
    let mut c = h.client().unwrap();

    let root_str = scratch.path().to_string_lossy().into_owned();
    c.call("watch-project", [Value::String(root_str.clone())])
        .unwrap();

    // Confirm it actually registered before we start reaping.
    assert!(
        listed_roots(&mut c).iter().any(|p| p == &root_str),
        "root missing after watch-project"
    );

    // Kill the directory behind the daemon's back.  On macOS the
    // canonical path lives under /private/var/folders; unlink both
    // forms so fsevents can't resurrect it.
    std::fs::remove_dir_all(scratch.path()).expect("removing scratch dir");

    // First tick: records the miss, sets `missing_ticks = 1`, no reap.
    c.call("debug-gc-tick", []).unwrap();
    assert!(
        listed_roots(&mut c).iter().any(|p| p == &root_str),
        "root reaped after a single missing tick; grace period broken"
    );

    // Second tick: threshold met, reap.
    c.call("debug-gc-tick", []).unwrap();
    let after = listed_roots(&mut c);
    assert!(
        !after.iter().any(|p| p == &root_str),
        "dead root still present after two GC ticks: {after:?}"
    );

    // And the status report should show it in the reap log.
    let status = c.call("status", []).unwrap();
    let reaped = status
        .as_object()
        .and_then(|o| o.get("reaped"))
        .and_then(Value::as_array)
        .expect("status response missing `reaped` array");
    assert!(
        reaped.iter().any(|entry| {
            let Some(o) = entry.as_object() else {
                return false;
            };
            o.get("path").and_then(Value::as_str) == Some(root_str.as_str())
                && o.get("reason").and_then(Value::as_str) == Some("dead")
        }),
        "reap log missing the dead root: {reaped:?}"
    );
}

/// An active root — one with a live subscription — must never be
/// reaped, even if its idle counter is old.  This is the one place
/// the zero-conf policy has to defend against: something flashy
/// happening in the file tree shouldn't matter, only "is anyone
/// actually listening".
#[test]
fn root_with_subscription_survives_gc() {
    let h = Harness::spawn().expect("spawn daemon");
    let scratch = Scratch::new().unwrap();
    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());

    c.call("watch-project", [root.clone()]).unwrap();
    c.call(
        "subscribe",
        [
            root.clone(),
            Value::String("test-sub".into()),
            Value::Object(indexmap::IndexMap::new()),
        ],
    )
    .unwrap();

    // Several ticks of nothing changing.
    for _ in 0..3 {
        c.call("debug-gc-tick", []).unwrap();
    }

    let roots = listed_roots(&mut c);
    let root_str = scratch.path().to_string_lossy().into_owned();
    assert!(
        roots.iter().any(|p| p == &root_str),
        "subscribed root was reaped: {roots:?}"
    );
}

/// The `status` command returns the shape the pretty-printer expects —
/// including the memory-breakdown object and per-root live/tombstone
/// counts added alongside aggressive tombstone pruning.
#[test]
fn status_reports_roots_and_counters() {
    let h = Harness::spawn().expect("spawn daemon");
    let scratch = Scratch::new().unwrap();
    let mut c = h.client().unwrap();

    c.call(
        "watch-project",
        [Value::String(scratch.path().to_string_lossy().into())],
    )
    .unwrap();

    let resp = c.call("status", []).unwrap();
    let obj = resp.as_object().expect("status not an object");

    for key in [
        "pid",
        "uptime_seconds",
        "rss_bytes",
        "total_tracked_files",
        "total_live_files",
        "total_tombstones",
        "memory",
        "roots",
        "reaped",
    ] {
        assert!(obj.contains_key(key), "status missing key `{key}`: {obj:?}");
    }

    let mem = obj.get("memory").and_then(Value::as_object).unwrap();
    for key in [
        "rss_bytes",
        "tree_bytes_est",
        "unaccounted_bytes",
        "live_entries",
        "tombstone_entries",
        "entry_size_bytes",
    ] {
        assert!(
            mem.contains_key(key),
            "memory breakdown missing `{key}`: {mem:?}"
        );
    }

    let roots = obj.get("roots").and_then(Value::as_array).unwrap();
    assert_eq!(roots.len(), 1, "expected 1 root in status");
    let root = roots[0].as_object().unwrap();
    for key in [
        "path",
        "num_files",
        "live_files",
        "tombstones",
        "tree_bytes_est",
        "idle_seconds",
        "health",
    ] {
        assert!(
            root.contains_key(key),
            "root entry missing `{key}`: {root:?}"
        );
    }

    // Fresh scratch root has no tombstones; a freshly-watched empty
    // dir should report zero.
    assert_eq!(
        root.get("tombstones").and_then(Value::as_i64),
        Some(0),
        "fresh watch shouldn't carry tombstones"
    );
}

/// `debug-ageout` used to be a documented no-op; it now triggers the
/// same tombstone sweep the GC runs. On an empty root it has nothing
/// to free, but it should still succeed and return the expected shape.
#[test]
fn debug_ageout_runs_tombstone_sweep() {
    let h = Harness::spawn().expect("spawn daemon");
    let scratch = Scratch::new().unwrap();
    let mut c = h.client().unwrap();
    c.call(
        "watch-project",
        [Value::String(scratch.path().to_string_lossy().into())],
    )
    .unwrap();

    let resp = c.call("debug-ageout", [Value::Int(0)]).unwrap();
    let obj = resp.as_object().expect("ageout not an object");
    assert_eq!(obj.get("ageout").and_then(Value::as_bool), Some(true));
    assert_eq!(obj.get("files").and_then(Value::as_i64), Some(0));
}

fn listed_roots(c: &mut watchwoman_tests::Client) -> Vec<String> {
    let list = c.call("watch-list", []).unwrap();
    list.as_object()
        .and_then(|o| o.get("roots"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
