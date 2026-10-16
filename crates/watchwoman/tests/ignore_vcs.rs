use std::time::{Duration, Instant};

use indexmap::IndexMap;
use watchwoman_protocol::Value;
use watchwoman_tests::{Harness, Scratch};

#[test]
fn configured_vcs_directories_are_watched_shallowly() {
    let h = Harness::spawn().expect("spawn daemon");
    let scratch = Scratch::new().unwrap();
    scratch
        .write(".watchmanconfig", br#"{"ignore_vcs":[".custom-vcs"]}"#)
        .unwrap();
    scratch.write(".custom-vcs/repo/store/hidden", b"").unwrap();
    scratch.write(".git/objects/visible", b"").unwrap();

    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();

    let mut query = IndexMap::new();
    query.insert(
        "fields".into(),
        Value::Array(vec![Value::String("name".into())]),
    );
    let query = Value::Object(query);
    let mut names = || {
        let response = c.call("query", [root.clone(), query.clone()]).unwrap();
        response
            .as_object()
            .unwrap()
            .get("files")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };

    let initial = names();
    assert!(initial.iter().any(|name| name == ".custom-vcs/repo"));
    assert!(!initial.iter().any(|name| name == ".custom-vcs/repo/store"));
    assert!(initial.iter().any(|name| name == ".git/objects/visible"));

    scratch.write(".custom-vcs/repo/late", b"").unwrap();
    scratch.write("visible.txt", b"").unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut observed = names();
    while !observed.iter().any(|name| name == "visible.txt") && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        observed = names();
    }
    assert!(observed.iter().any(|name| name == "visible.txt"));
    // The two writes may land in separate batches; give the first a chance
    // to arrive before asserting it was ignored.
    std::thread::sleep(Duration::from_millis(200));
    observed = names();
    assert!(!observed.iter().any(|name| name == ".custom-vcs/repo/late"));
}
