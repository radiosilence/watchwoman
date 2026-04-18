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
fn state_enter_and_leave_round_trip() {
    let h = Harness::spawn().expect("spawn daemon");
    let scratch = Scratch::new().unwrap();
    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();

    let enter = c
        .call(
            "state-enter",
            [
                root.clone(),
                obj(&[("name", Value::String("build".into()))]),
            ],
        )
        .unwrap();
    assert!(enter
        .as_object()
        .is_some_and(|o| o.contains_key("state-enter")));

    let leave = c
        .call(
            "state-leave",
            [root, obj(&[("name", Value::String("build".into()))])],
        )
        .unwrap();
    assert!(leave
        .as_object()
        .is_some_and(|o| o.contains_key("state-leave")));
}
