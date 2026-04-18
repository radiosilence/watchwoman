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
fn subscribe_emits_changes() {
    let Ok(h) = Harness::spawn() else { return };
    let scratch = Scratch::new().unwrap();
    let mut c = h.client().unwrap();

    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();

    let sub = obj(&[
        ("fields", Value::Array(vec![Value::String("name".into())])),
        ("empty_on_fresh_instance", Value::Bool(true)),
    ]);
    let resp = c
        .call("subscribe", [root.clone(), Value::String("t".into()), sub])
        .unwrap();
    assert!(resp
        .as_object()
        .is_some_and(|o| o.contains_key("subscribe")));

    scratch.write("new.txt", b"hello").unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got_name = false;
    while Instant::now() < deadline {
        let Some(pdu) = c.read_unilateral().unwrap() else {
            break;
        };
        if let Some(files) = pdu
            .as_object()
            .and_then(|o| o.get("files"))
            .and_then(Value::as_array)
        {
            for f in files {
                if let Some(name) = f
                    .as_object()
                    .and_then(|o| o.get("name"))
                    .and_then(Value::as_str)
                {
                    if name == "new.txt" {
                        got_name = true;
                        break;
                    }
                }
            }
            if got_name {
                break;
            }
        }
    }
    assert!(got_name, "subscription did not surface new.txt");
}
