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
fn query_with_suffix_matches_expected_files() {
    let Ok(h) = Harness::spawn() else { return };
    let scratch = Scratch::new().unwrap();
    scratch.write("src/main.rs", b"fn main() {}").unwrap();
    scratch.write("README.md", b"# hi").unwrap();
    scratch.write("crates/foo/lib.rs", b"").unwrap();

    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();

    let q = obj(&[
        ("suffix", Value::Array(vec![Value::String("rs".into())])),
        ("fields", Value::Array(vec![Value::String("name".into())])),
    ]);
    let resp = c.call("query", [root, q]).unwrap();
    let files = resp
        .as_object()
        .and_then(|o| o.get("files"))
        .and_then(Value::as_array)
        .expect("no files");

    let names: Vec<&str> = files
        .iter()
        .filter_map(|f| {
            f.as_object()
                .and_then(|o| o.get("name"))
                .and_then(Value::as_str)
        })
        .collect();
    for expected in ["src/main.rs", "crates/foo/lib.rs"] {
        assert!(names.contains(&expected), "missing {expected} in {names:?}");
    }
    assert!(
        !names.contains(&"README.md"),
        "README.md should not match suffix rs: {names:?}"
    );
}

#[test]
fn query_match_glob_excludes_other_suffixes() {
    let Ok(h) = Harness::spawn() else { return };
    let scratch = Scratch::new().unwrap();
    scratch.write("a.txt", b"").unwrap();
    scratch.write("b.md", b"").unwrap();

    let mut c = h.client().unwrap();
    let root = Value::String(scratch.path().to_string_lossy().into());
    c.call("watch-project", [root.clone()]).unwrap();

    let expr = Value::Array(vec![
        Value::String("match".into()),
        Value::String("*.md".into()),
        Value::String("basename".into()),
    ]);
    let q = obj(&[
        ("expression", expr),
        ("fields", Value::Array(vec![Value::String("name".into())])),
    ]);
    let resp = c.call("query", [root, q]).unwrap();
    let files = resp
        .as_object()
        .and_then(|o| o.get("files"))
        .and_then(Value::as_array)
        .expect("no files");

    let names: Vec<&str> = files
        .iter()
        .filter_map(|f| {
            f.as_object()
                .and_then(|o| o.get("name"))
                .and_then(Value::as_str)
        })
        .collect();
    assert_eq!(names, vec!["b.md"], "unexpected match set: {names:?}");
}
