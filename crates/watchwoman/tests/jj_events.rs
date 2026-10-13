use std::time::{Duration, Instant};

use indexmap::IndexMap;
use watchwoman_protocol::Value;
use watchwoman_tests::{Harness, Scratch, TargetBinary};

fn jj_query(since: Option<&str>) -> Value {
    let exclude = Value::Array(vec![
        Value::String("not".into()),
        Value::Array(vec![
            Value::String("anyof".into()),
            Value::Array(vec![
                Value::String("name".into()),
                Value::Array(vec![
                    Value::String(".git".into()),
                    Value::String(".jj".into()),
                ]),
                Value::String("wholename".into()),
            ]),
            Value::Array(vec![
                Value::String("dirname".into()),
                Value::String(".git".into()),
            ]),
            Value::Array(vec![
                Value::String("dirname".into()),
                Value::String(".jj".into()),
            ]),
        ]),
    ]);
    let mut spec = IndexMap::new();
    spec.insert("expression".into(), exclude);
    spec.insert(
        "fields".into(),
        Value::Array(vec![Value::String("name".into())]),
    );
    if let Some(since) = since {
        spec.insert("since".into(), Value::String(since.into()));
    }
    Value::Object(spec)
}

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

    // A crawl would have indexed these; the event index never does.
    // Events for the other files written above may still arrive late.
    assert_eq!(index_mode(&mut c), Value::String("jj-events".into()));
    let initial = c.call("query", [root.clone(), jj_query(None)]).unwrap();
    let initial = initial.as_object().unwrap();
    assert_eq!(initial.get("is_fresh_instance"), Some(&Value::Bool(true)));
    let names: Vec<_> = initial
        .get("files")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(!names.contains(&"generated/old"), "{names:?}");
    assert!(!names.contains(&".jj/marker"), "{names:?}");
    let clock = settled_clock(&mut c, &root, &scratch);

    scratch.write("new.txt", b"new").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = c
            .call("query", [root.clone(), jj_query(Some(&clock))])
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

    let stale = c
        .call("query", [root, jj_query(Some("c:0:0:0:0"))])
        .unwrap();
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

    let response = c.call("query", [root, jj_query(Some(clock))]).unwrap();
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
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = c
            .call("query", [root.clone(), jj_query(Some(clock))])
            .unwrap();
        let files = response
            .as_object()
            .unwrap()
            .get("files")
            .unwrap()
            .as_array()
            .unwrap();
        if files
            .iter()
            .any(|file| file.as_str() == Some("tracked.txt"))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "preexisting deletion was not reported"
        );
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
    let clock = settled_clock(&mut c, &root, &scratch);

    scratch.write("generated/one", b"new").unwrap();
    scratch.write("generated/two", b"new").unwrap();
    scratch.write("src/cache/one", b"new").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let collapsed_clock = loop {
        let response = c
            .call("query", [root.clone(), jj_query(Some(&clock))])
            .unwrap();
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
            .call("query", [root.clone(), jj_query(Some(&collapsed_clock))])
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

#[test]
fn a_general_watchman_query_promotes_a_jj_root_to_a_full_index() {
    let h = Harness::spawn_with(TargetBinary::Watchwoman).unwrap();
    let scratch = Scratch::new().unwrap();
    scratch.write(".jj/marker", b"").unwrap();
    scratch.write("preexisting.txt", b"old").unwrap();
    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();

    let mut spec = IndexMap::new();
    spec.insert(
        "fields".into(),
        Value::Array(vec![Value::String("name".into())]),
    );
    let result = c.call("query", [root, Value::Object(spec)]).unwrap();
    assert!(result
        .as_object()
        .unwrap()
        .get("files")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .any(|file| file.as_str() == Some("preexisting.txt")));
    let status = c.call("status", []).unwrap();
    let roots = status
        .as_object()
        .unwrap()
        .get("roots")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(
        roots[0].as_object().unwrap().get("index_mode"),
        Some(&Value::String("full".into()))
    );
}

fn clock_of(c: &mut watchwoman_tests::Client, root: &Value) -> String {
    let response = c.call("clock", [root.clone()]).unwrap();
    response
        .as_object()
        .unwrap()
        .get("clock")
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

fn index_mode(c: &mut watchwoman_tests::Client) -> Value {
    let status = c.call("status", []).unwrap();
    status
        .as_object()
        .unwrap()
        .get("roots")
        .unwrap()
        .as_array()
        .unwrap()[0]
        .as_object()
        .unwrap()
        .get("index_mode")
        .unwrap()
        .clone()
}

/// Delete `name` and wait until a jj query since `clock` reports it gone.
fn delete_and_wait(
    c: &mut watchwoman_tests::Client,
    root: &Value,
    scratch: &Scratch,
    name: &str,
    clock: &str,
) {
    std::fs::remove_file(scratch.path().join(name)).unwrap();
    let Value::Object(mut spec) = jj_query(Some(clock)) else {
        unreachable!()
    };
    spec.insert(
        "fields".into(),
        Value::Array(vec![
            Value::String("name".into()),
            Value::String("exists".into()),
        ]),
    );
    let spec = Value::Object(spec);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = c.call("query", [root.clone(), spec.clone()]).unwrap();
        let files = response
            .as_object()
            .unwrap()
            .get("files")
            .unwrap()
            .as_array()
            .unwrap();
        let gone = files.iter().filter_map(Value::as_object).any(|file| {
            file.get("name").and_then(Value::as_str) == Some(name)
                && file.get("exists") == Some(&Value::Bool(false))
        });
        if gone {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "deletion of {name} was not reported"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn promotion_answers_clocks_from_the_event_index_afresh() {
    let h = Harness::spawn_with(TargetBinary::Watchwoman).unwrap();
    let scratch = Scratch::new().unwrap();
    scratch.write(".jj/marker", b"").unwrap();
    scratch.write("tracked.txt", b"preexisting").unwrap();
    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();
    let clock = settled_clock(&mut c, &root, &scratch);
    delete_and_wait(&mut c, &root, &scratch, "tracked.txt", &clock);

    let mut spec = IndexMap::new();
    spec.insert(
        "fields".into(),
        Value::Array(vec![Value::String("name".into())]),
    );
    c.call("query", [root.clone(), Value::Object(spec)])
        .unwrap();
    assert_eq!(index_mode(&mut c), Value::String("full".into()));

    // The crawl cannot know about the deletion, so the old clock must not
    // be answered as if nothing had been lost.
    let response = c.call("query", [root, jj_query(Some(&clock))]).unwrap();
    assert_eq!(
        response.as_object().unwrap().get("is_fresh_instance"),
        Some(&Value::Bool(true))
    );
}

#[test]
fn reloaded_general_triggers_promote_a_jj_root() {
    let h = Harness::spawn_with(TargetBinary::Watchwoman).unwrap();
    let scratch = Scratch::new().unwrap();
    scratch.write(".jj/marker", b"").unwrap();
    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();
    let mut spec = IndexMap::new();
    spec.insert("name".into(), Value::String("build".into()));
    spec.insert(
        "command".into(),
        Value::Array(vec![Value::String("true".into())]),
    );
    c.call("trigger", [root.clone(), Value::Object(spec)])
        .unwrap();

    // Re-registering the root reloads its persisted triggers, as a daemon
    // restart does.
    c.call("watch-del", [root.clone()]).unwrap();
    c.call("watch-project", [root]).unwrap();
    assert_eq!(index_mode(&mut c), Value::String("full".into()));
}

#[test]
fn tombstone_sweeps_leave_jj_clocks_answerable() {
    let h = Harness::spawn_with(TargetBinary::Watchwoman).unwrap();
    let scratch = Scratch::new().unwrap();
    scratch.write(".jj/marker", b"").unwrap();
    scratch.write("tracked.txt", b"preexisting").unwrap();
    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();
    let clock = settled_clock(&mut c, &root, &scratch);
    delete_and_wait(&mut c, &root, &scratch, "tracked.txt", &clock);

    c.call("debug-ageout", [Value::Int(0)]).unwrap();
    let response = c.call("query", [root, jj_query(Some(&clock))]).unwrap();
    let result = response.as_object().unwrap();
    assert_eq!(result.get("is_fresh_instance"), Some(&Value::Bool(false)));
    assert!(result
        .get("files")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .any(|file| file.as_str() == Some("tracked.txt")));
}

/// A clock after which the index has settled. Events for files written
/// just before the watch registered can still arrive, and a late
/// `.gitignore` among them invalidates the index. FSEvents and inotify
/// deliver in order, so once a file written now is reported incrementally,
/// those earlier events are in.
fn settled_clock(c: &mut watchwoman_tests::Client, root: &Value, scratch: &Scratch) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut clock = clock_of(c, root);
    for n in 0.. {
        let sentinel = format!("settle-{n}");
        scratch.write(&sentinel, b"").unwrap();
        loop {
            let response = c
                .call("query", [root.clone(), jj_query(Some(&clock))])
                .unwrap();
            let result = response.as_object().unwrap();
            let next = result.get("clock").unwrap().as_str().unwrap().to_owned();
            if result.get("is_fresh_instance") == Some(&Value::Bool(true)) {
                clock = next;
                break;
            }
            let files = result.get("files").unwrap().as_array().unwrap();
            if files
                .iter()
                .any(|file| file.as_str() == Some(sentinel.as_str()))
            {
                return next;
            }
            assert!(Instant::now() < deadline, "the index did not settle");
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    unreachable!()
}

#[test]
fn a_gitignore_inside_an_ignored_directory_keeps_the_index() {
    let h = Harness::spawn_with(TargetBinary::Watchwoman).unwrap();
    let scratch = Scratch::new().unwrap();
    scratch.write(".jj/marker", b"").unwrap();
    scratch.write(".gitignore", b"/node_modules/\n").unwrap();
    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();
    let clock = settled_clock(&mut c, &root, &scratch);

    scratch
        .write("node_modules/pkg/.gitignore", b"dist\n")
        .unwrap();
    scratch.write("tracked.txt", b"new").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = c
            .call("query", [root.clone(), jj_query(Some(&clock))])
            .unwrap();
        let result = response.as_object().unwrap();
        assert_eq!(
            result.get("is_fresh_instance"),
            Some(&Value::Bool(false)),
            "a package's .gitignore invalidated the index"
        );
        let files = result.get("files").unwrap().as_array().unwrap();
        let names: Vec<_> = files.iter().filter_map(Value::as_str).collect();
        if names.contains(&"node_modules/.gitignore") && names.contains(&"tracked.txt") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "changes were not reported: {names:?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn scm_clocks_are_not_fresh_after_promotion() {
    let h = Harness::spawn_with(TargetBinary::Watchwoman).unwrap();
    let scratch = Scratch::new().unwrap();
    scratch.write(".jj/marker", b"").unwrap();
    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();

    let mut spec = IndexMap::new();
    spec.insert(
        "fields".into(),
        Value::Array(vec![Value::String("name".into())]),
    );
    c.call("query", [root.clone(), Value::Object(spec.clone())])
        .unwrap();
    assert_eq!(index_mode(&mut c), Value::String("full".into()));

    // A mergebase query filters by the paths git reports, not by tick, so
    // the promotion's tick has no bearing on it.
    spec.insert("since".into(), Value::String("scm:git:HEAD".into()));
    spec.insert("empty_on_fresh_instance".into(), Value::Bool(true));
    let response = c.call("query", [root, Value::Object(spec)]).unwrap();
    assert_eq!(
        response.as_object().unwrap().get("is_fresh_instance"),
        Some(&Value::Bool(false))
    );
}
