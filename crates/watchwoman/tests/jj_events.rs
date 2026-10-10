use std::time::{Duration, Instant};

use indexmap::IndexMap;
use watchwoman_protocol::Value;
use watchwoman_tests::{Harness, Scratch, TargetBinary};

#[test]
fn jj_mode_starts_without_crawling_and_reports_later_changes() {
    let h = Harness::spawn_with(TargetBinary::Watchwoman).unwrap();
    let scratch = Scratch::new().unwrap();
    scratch
        .write(".watchmanconfig", br#"{"watchwoman_vcs":"jj"}"#)
        .unwrap();
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

    let stale = c
        .call("query", [root, query(Some("c:0:0:0:0"))])
        .unwrap();
    assert_eq!(
        stale.as_object().unwrap().get("is_fresh_instance"),
        Some(&Value::Bool(true))
    );
}
