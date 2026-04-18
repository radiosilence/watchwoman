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
fn query_returns_content_sha1hex() {
    // Known SHA-1 of "hello\n" (lowercase hex, 40 chars).
    const HELLO_SHA1: &str = "f572d396fae9206628714fb2ce00f72e94f2258f";

    let h = Harness::spawn().expect("spawn daemon");
    let scratch = Scratch::new().unwrap();
    scratch.write("hello.txt", b"hello\n").unwrap();

    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();

    let q = obj(&[
        (
            "fields",
            Value::Array(vec![
                Value::String("name".into()),
                Value::String("content.sha1hex".into()),
            ]),
        ),
        (
            "expression",
            Value::Array(vec![
                Value::String("name".into()),
                Value::String("hello.txt".into()),
            ]),
        ),
    ]);
    let resp = c.call("query", [root, q]).unwrap();
    let files = resp
        .as_object()
        .and_then(|o| o.get("files"))
        .and_then(Value::as_array)
        .expect("files");
    let row = files.first().expect("one row");
    let sha = row
        .as_object()
        .and_then(|o| o.get("content.sha1hex"))
        .and_then(Value::as_str)
        .expect("sha1hex field");
    assert_eq!(sha, HELLO_SHA1);
}
