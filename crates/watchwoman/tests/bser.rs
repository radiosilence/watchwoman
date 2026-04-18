//! Round-trip tests for the BSER wire path.
//!
//! Hits the daemon directly on its unix socket using BSER v1 and v2
//! framing and asserts the response is a valid BSER PDU with the
//! expected shape.

use std::io::{BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use watchwoman_protocol::{bser, Encoding, Value};
use watchwoman_tests::{Harness, Scratch};

fn open(sock: &std::path::Path) -> UnixStream {
    let s = UnixStream::connect(sock).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    s
}

fn send_and_recv(stream: &mut UnixStream, pdu: &Value, encoding: Encoding) -> Value {
    let bytes = bser::encode_pdu(pdu, encoding).expect("encode");
    stream.write_all(&bytes).unwrap();
    stream.flush().unwrap();
    let mut reader = BufReader::new(stream);
    let (value, _) = bser::read_pdu(&mut reader).expect("read").expect("value");
    value
}

#[test]
fn bser_v1_version_roundtrip() {
    let h = Harness::spawn().expect("spawn daemon");
    let mut s = open(h.sock());

    let pdu = Value::Array(vec![Value::String("version".into())]);
    let resp = send_and_recv(&mut s, &pdu, Encoding::BserV1);

    let obj = resp.as_object().expect("object response");
    assert!(
        obj.contains_key("version"),
        "response missing `version`: {obj:?}"
    );
}

#[test]
fn bser_v2_watch_project_roundtrip() {
    let h = Harness::spawn().expect("spawn daemon");
    let scratch = Scratch::new().unwrap();
    let mut s = open(h.sock());

    let pdu = Value::Array(vec![
        Value::String("watch-project".into()),
        Value::String(scratch.path().to_string_lossy().into()),
    ]);
    let resp = send_and_recv(&mut s, &pdu, Encoding::BserV2);

    let obj = resp.as_object().expect("object response");
    let watch = obj.get("watch").and_then(Value::as_str).expect("watch");
    assert_eq!(std::path::Path::new(watch), scratch.path());
}
