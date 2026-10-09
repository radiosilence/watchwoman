use std::io::Write;
use std::process::{Command, Stdio};

use watchwoman_protocol::{bser, Encoding, Value};
use watchwoman_tests::Harness;

#[test]
fn cli_encodes_socket_discovery_as_bser() {
    let h = Harness::spawn().expect("spawn daemon");
    for (name, encoding) in [
        ("bser", Encoding::BserV1),
        ("bser2", Encoding::BserV2),
        ("bser-v2", Encoding::BserV2),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_watchman"))
            .arg("--sockname")
            .arg(h.sock())
            .args(["--no-spawn", "--output-encoding", name, "get-sockname"])
            .output()
            .expect("run CLI");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut bytes = output.stdout.as_slice();
        let (value, actual) = bser::read_pdu(&mut bytes)
            .expect("decode BSER")
            .expect("response");
        assert_eq!(actual, encoding);
        assert!(bytes.is_empty(), "unexpected bytes after PDU");
        assert_eq!(
            value.as_object().unwrap()["sockname"].as_str(),
            h.sock().to_str()
        );
    }
}

#[test]
fn cli_encodes_stdin_responses_and_errors_as_bser() {
    let h = Harness::spawn().expect("spawn daemon");
    for (request, success) in [(r#"["version"]"#, true), (r#"["not-a-command"]"#, false)] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_watchman"))
            .arg("--sockname")
            .arg(h.sock())
            .args(["--no-spawn", "--output-encoding=bser-v2", "-j"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run CLI");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(request.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            output.status.success(),
            success,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let (value, used) = bser::decode_pdu(&output.stdout).expect("decode BSER");
        assert_eq!(used, output.stdout.len());
        let object = value.as_object().unwrap();
        assert!(object
            .get(if success { "version" } else { "error" })
            .and_then(Value::as_str)
            .is_some());
    }
}

#[test]
fn cli_status_honors_binary_output_and_preserves_json_output() {
    let h = Harness::spawn().expect("spawn daemon");
    for encoding in ["json", "bser-v2"] {
        let output = Command::new(env!("CARGO_BIN_EXE_watchman"))
            .arg("--sockname")
            .arg(h.sock())
            .args([
                "--no-spawn",
                "--output-encoding",
                encoding,
                "status",
                "--json",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        if encoding == "json" {
            let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(value.is_object());
        } else {
            let (value, used) = bser::decode_pdu(&output.stdout).unwrap();
            assert!(value.as_object().is_some());
            assert_eq!(used, output.stdout.len());
        }
    }
}

#[test]
fn cli_encodes_each_persistent_response() {
    use std::io::BufReader;
    use std::os::unix::net::UnixListener;
    use watchwoman_protocol::json;
    use watchwoman_tests::Scratch;

    let scratch = Scratch::new().unwrap();
    let socket = scratch.path().join("sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let request = json::read_pdu(&mut BufReader::new(stream.try_clone().unwrap())).unwrap();
        assert!(request.is_some());
        for value in [
            Value::String("first".into()),
            Value::String("second".into()),
        ] {
            json::encode_pdu(&mut stream, &value).unwrap();
        }
    });
    let output = Command::new(env!("CARGO_BIN_EXE_watchman"))
        .arg("--sockname")
        .arg(&socket)
        .args([
            "--no-spawn",
            "--persistent",
            "--output-encoding=bser-v2",
            "version",
        ])
        .output()
        .unwrap();
    server.join().unwrap();
    assert!(output.status.success());
    let mut bytes = output.stdout.as_slice();
    for expected in ["first", "second"] {
        let (value, encoding) = bser::read_pdu(&mut bytes).unwrap().unwrap();
        assert_eq!(encoding, Encoding::BserV2);
        assert_eq!(value.as_str(), Some(expected));
    }
    assert!(bytes.is_empty());
}
