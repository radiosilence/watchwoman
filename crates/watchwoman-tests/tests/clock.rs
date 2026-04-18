use watchwoman_protocol::Value;
use watchwoman_tests::{Harness, Scratch};

#[test]
fn clock_monotonic_under_writes() {
    let Ok(h) = Harness::spawn() else { return };
    let scratch = Scratch::new().unwrap();
    let mut c = h.client().unwrap();

    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();

    let first = c.call("clock", [root.clone()]).unwrap();
    let first_clock = first
        .as_object()
        .and_then(|o| o.get("clock"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .expect("no clock");

    scratch.write("poke.txt", b"first").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(250));

    let second = c.call("clock", [root]).unwrap();
    let second_clock = second
        .as_object()
        .and_then(|o| o.get("clock"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .expect("no clock");

    assert!(
        first_clock != second_clock,
        "clock unchanged after write: {first_clock}"
    );
}
