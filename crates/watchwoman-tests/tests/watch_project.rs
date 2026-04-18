use watchwoman_protocol::Value;
use watchwoman_tests::{Harness, Scratch};

#[test]
fn watch_project_returns_root() {
    let Ok(h) = Harness::spawn() else {
        eprintln!("skipping: harness unavailable");
        return;
    };
    let scratch = Scratch::new().unwrap();
    let mut c = h.client().unwrap();

    let resp = c
        .call(
            "watch-project",
            [Value::String(scratch.path().to_string_lossy().into())],
        )
        .unwrap();
    let obj = resp.as_object().expect("non-object response");
    let watch = obj.get("watch").and_then(Value::as_str).expect("no watch");
    assert_eq!(std::path::Path::new(watch), scratch.path());
}

#[test]
fn watch_list_contains_roots() {
    let Ok(h) = Harness::spawn() else { return };
    let scratch = Scratch::new().unwrap();
    let mut c = h.client().unwrap();

    c.call(
        "watch-project",
        [Value::String(scratch.path().to_string_lossy().into())],
    )
    .unwrap();

    let list = c.call("watch-list", []).unwrap();
    let roots = list
        .as_object()
        .and_then(|o| o.get("roots"))
        .and_then(Value::as_array)
        .expect("no roots array");
    let paths: Vec<&str> = roots.iter().filter_map(Value::as_str).collect();
    assert!(
        paths
            .iter()
            .any(|p| std::path::Path::new(p) == scratch.path()),
        "scratch not in roots: {paths:?}"
    );
}

#[test]
fn watch_del_removes_root() {
    let Ok(h) = Harness::spawn() else { return };
    let scratch = Scratch::new().unwrap();
    let mut c = h.client().unwrap();

    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();
    c.call("watch-del", [root]).unwrap();

    let list = c.call("watch-list", []).unwrap();
    let roots = list
        .as_object()
        .and_then(|o| o.get("roots"))
        .and_then(Value::as_array)
        .expect("no roots array");
    let paths: Vec<&str> = roots.iter().filter_map(Value::as_str).collect();
    assert!(
        !paths
            .iter()
            .any(|p| std::path::Path::new(p) == scratch.path()),
        "scratch still in roots after watch-del: {paths:?}"
    );
}
