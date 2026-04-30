use std::time::{Duration, Instant};

use indexmap::IndexMap;
use watchwoman_protocol::Value;
use watchwoman_tests::{Harness, Scratch};

fn obj(entries: &[(&str, Value)]) -> Value {
    let mut m = IndexMap::with_capacity(entries.len());
    for (k, v) in entries {
        m.insert((*k).to_owned(), v.clone());
    }
    Value::Object(m)
}

#[test]
fn trigger_fires_on_change() {
    let h = Harness::spawn().expect("spawn daemon");
    let scratch = Scratch::new().unwrap();
    let mut c = h.client().unwrap();

    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();

    let marker = scratch.path().join("trigger-fired");
    let spec = obj(&[
        ("name", Value::String("rs-touch".into())),
        (
            "command",
            Value::Array(vec![
                Value::String("/bin/sh".into()),
                Value::String("-c".into()),
                Value::String(format!("touch {}", marker.display())),
            ]),
        ),
        (
            "expression",
            Value::Array(vec![
                Value::String("suffix".into()),
                Value::String("rs".into()),
            ]),
        ),
    ]);
    let resp = c.call("trigger", [root.clone(), spec]).unwrap();
    assert_eq!(
        resp.as_object()
            .and_then(|o| o.get("trigger"))
            .and_then(Value::as_str),
        Some("rs-touch")
    );

    scratch.write("main.rs", b"fn main() {}").unwrap();

    // CI runners (especially the linux-arm and macos-amd64 cross
    // slices) routinely take 5–10 s to wake the trigger task — fork
    // + `/bin/sh -c touch` + watcher debounce all stack up and the
    // 50 ms poll loop misses narrowly.  A genuine regression keeps
    // the marker absent forever, so a generous deadline doesn't hide
    // bugs, only cuts the false-positive rate.
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut fired = false;
    while Instant::now() < deadline {
        if marker.exists() {
            fired = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(fired, "trigger did not execute; marker not created");
}

#[test]
fn trigger_list_and_del() {
    let h = Harness::spawn().expect("spawn daemon");
    let scratch = Scratch::new().unwrap();
    let mut c = h.client().unwrap();

    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();
    let spec = obj(&[
        ("name", Value::String("t1".into())),
        (
            "command",
            Value::Array(vec![Value::String("/bin/true".into())]),
        ),
    ]);
    c.call("trigger", [root.clone(), spec]).unwrap();

    let list = c.call("trigger-list", [root.clone()]).unwrap();
    let triggers = list
        .as_object()
        .and_then(|o| o.get("triggers"))
        .and_then(Value::as_array)
        .expect("triggers");
    assert_eq!(triggers.len(), 1);

    let del = c
        .call("trigger-del", [root.clone(), Value::String("t1".into())])
        .unwrap();
    assert_eq!(
        del.as_object()
            .and_then(|o| o.get("deleted"))
            .and_then(Value::as_i64),
        Some(1)
    );
    let list2 = c.call("trigger-list", [root]).unwrap();
    let triggers2 = list2
        .as_object()
        .and_then(|o| o.get("triggers"))
        .and_then(Value::as_array)
        .expect("triggers");
    assert!(triggers2.is_empty());
}
