use std::time::{Duration, Instant};

use indexmap::IndexMap;
use watchwoman_protocol::Value;
use watchwoman_tests::{Harness, Scratch, TargetBinary};

#[test]
fn jj_mode_starts_without_crawling_and_reports_later_changes() {
    let h = Harness::spawn_with(TargetBinary::Watchwoman).unwrap();
    let scratch = Scratch::new().unwrap();
    scratch.write(".jj/marker", b"").unwrap();
    scratch.write(".gitignore", b"/generated/\n").unwrap();
    scratch.write("generated/old", b"ignored by jj").unwrap();
    scratch.write("tracked.txt", b"tracked by jj").unwrap();

    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();

    let status = c.call("status", []).unwrap();
    let roots = status
        .as_object()
        .unwrap()
        .get("roots")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        roots[0]
            .as_object()
            .unwrap()
            .get("num_files")
            .and_then(Value::as_i64),
        Some(0)
    );
    assert_eq!(
        roots[0].as_object().unwrap().get("index_mode"),
        Some(&Value::String("jj-events".into()))
    );

    let query = |since: Option<&str>| {
        let mut spec = IndexMap::new();
        spec.insert(
            "fields".into(),
            Value::Array(vec![Value::String("name".into())]),
        );
        if let Some(since) = since {
            spec.insert("since".into(), Value::String(since.into()));
        }
        Value::Object(spec)
    };
    let initial = c.call("query", [root.clone(), query(None)]).unwrap();
    let initial = initial.as_object().unwrap();
    assert_eq!(initial.get("is_fresh_instance"), Some(&Value::Bool(true)));
    assert!(initial.get("files").unwrap().as_array().unwrap().is_empty());
    let clock = initial.get("clock").unwrap().as_str().unwrap().to_owned();

    scratch.write("new.txt", b"new").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = c
            .call("query", [root.clone(), query(Some(&clock))])
            .unwrap();
        let result = response.as_object().unwrap();
        let files = result.get("files").unwrap().as_array().unwrap();
        if files.iter().any(|file| file.as_str() == Some("new.txt")) {
            assert_eq!(result.get("is_fresh_instance"), Some(&Value::Bool(false)));
            break;
        }
        assert!(Instant::now() < deadline, "new file event was not reported");
        std::thread::sleep(Duration::from_millis(25));
    }

    let stale = c.call("query", [root, query(Some("c:0:0:0:0"))]).unwrap();
    assert_eq!(
        stale.as_object().unwrap().get("is_fresh_instance"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn jj_mode_requires_full_rescan_after_event_loss() {
    let h = Harness::spawn_with(TargetBinary::Watchwoman).unwrap();
    let scratch = Scratch::new().unwrap();
    scratch.write(".jj/marker", b"").unwrap();
    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();
    let before = c.call("clock", [root.clone()]).unwrap();
    let clock = before
        .as_object()
        .unwrap()
        .get("clock")
        .unwrap()
        .as_str()
        .unwrap();

    let injected = c
        .call("debug-fsevents-inject-drop", [root.clone()])
        .unwrap();
    assert_eq!(
        injected.as_object().unwrap().get("injected"),
        Some(&Value::Bool(true))
    );

    let mut spec = IndexMap::new();
    spec.insert("since".into(), Value::String(clock.into()));
    let response = c.call("query", [root, Value::Object(spec)]).unwrap();
    assert_eq!(
        response.as_object().unwrap().get("is_fresh_instance"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn jj_mode_reports_deletion_of_a_file_it_never_indexed() {
    let h = Harness::spawn_with(TargetBinary::Watchwoman).unwrap();
    let scratch = Scratch::new().unwrap();
    scratch.write(".jj/marker", b"").unwrap();
    scratch.write("tracked.txt", b"preexisting").unwrap();
    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();
    let initial = c.call("clock", [root.clone()]).unwrap();
    let clock = initial
        .as_object()
        .unwrap()
        .get("clock")
        .unwrap()
        .as_str()
        .unwrap();

    std::fs::remove_file(scratch.path().join("tracked.txt")).unwrap();
    let mut spec = IndexMap::new();
    spec.insert("since".into(), Value::String(clock.into()));
    spec.insert("fields".into(), Value::Array(vec![Value::String("name".into())]));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = c
            .call("query", [root.clone(), Value::Object(spec.clone())])
            .unwrap();
        let files = response.as_object().unwrap().get("files").unwrap().as_array().unwrap();
        if files.iter().any(|file| file.as_str() == Some("tracked.txt")) {
            break;
        }
        assert!(Instant::now() < deadline, "preexisting deletion was not reported");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn jj_mode_collapses_recursively_ignored_directories_and_reloads_rules() {
    let h = Harness::spawn_with(TargetBinary::Watchwoman).unwrap();
    let scratch = Scratch::new().unwrap();
    scratch.write(".jj/marker", b"").unwrap();
    scratch.write(".gitignore", b"/generated/\n").unwrap();
    scratch.write("src/.gitignore", b"/cache/\n").unwrap();
    scratch.write("generated/existing", b"old").unwrap();
    scratch.write("src/cache/existing", b"old").unwrap();

    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();
    let initial = c.call("clock", [root.clone()]).unwrap();
    let clock = initial
        .as_object()
        .unwrap()
        .get("clock")
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();

    scratch.write("generated/one", b"new").unwrap();
    scratch.write("generated/two", b"new").unwrap();
    scratch.write("src/cache/one", b"new").unwrap();
    let query = |since: &str| {
        let mut spec = IndexMap::new();
        spec.insert("since".into(), Value::String(since.into()));
        spec.insert(
            "fields".into(),
            Value::Array(vec![Value::String("name".into())]),
        );
        Value::Object(spec)
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let collapsed_clock = loop {
        let response = c.call("query", [root.clone(), query(&clock)]).unwrap();
        let result = response.as_object().unwrap();
        let files = result.get("files").unwrap().as_array().unwrap();
        let names: Vec<_> = files.iter().filter_map(Value::as_str).collect();
        if names.contains(&"generated/.gitignore") && names.contains(&"src/cache/.gitignore") {
            assert!(!names.contains(&"generated/one"));
            assert!(!names.contains(&"generated/two"));
            assert!(!names.contains(&"src/cache/one"));
            break result.get("clock").unwrap().as_str().unwrap().to_owned();
        }
        assert!(
            Instant::now() < deadline,
            "ignored events were not collapsed: {names:?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    };

    scratch.write("src/.gitignore", b"").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = c
            .call("query", [root.clone(), query(&collapsed_clock)])
            .unwrap();
        if response.as_object().unwrap().get("is_fresh_instance") == Some(&Value::Bool(true)) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "ignore-rule change did not invalidate the index"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}
