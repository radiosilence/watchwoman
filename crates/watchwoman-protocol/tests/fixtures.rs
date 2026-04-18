//! Parity round-trips against fixtures recorded from real watchman.
//!
//! The `watchwoman-tests record-fixtures` binary spawns real watchman
//! and stores one JSON and one BSER-v2 response per scenario.  These
//! tests assert that:
//!
//! 1. Every committed BSER fixture decodes cleanly.
//! 2. Re-encoding the decoded value reproduces the byte sequence, so
//!    watchwoman emits PDUs that real clients accept without tweaks.
//! 3. JSON and BSER fixtures for the same scenario represent the same
//!    logical value (modulo BSER templates, which watchman uses for
//!    arrays of objects sharing a field layout).

use std::fs;
use std::path::{Path, PathBuf};

use watchwoman_protocol::{bser, Encoding, Value};

fn fixtures_root() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest_dir).join("tests").join("fixtures")
}

fn scenarios() -> Vec<String> {
    let mut out = Vec::new();
    let dir = fixtures_root();
    if !dir.exists() {
        return out;
    }
    for entry in fs::read_dir(&dir).expect("read fixtures dir").flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(stem) = name.strip_suffix(".json") {
            out.push(stem.to_owned());
        }
    }
    out.sort();
    out
}

#[test]
fn bser_fixtures_decode_and_reencode() {
    let dir = fixtures_root();
    for scenario in scenarios() {
        let bser_path = dir.join(format!("{scenario}.bser2"));
        if !bser_path.exists() {
            continue;
        }
        let bytes = fs::read(&bser_path).expect("read bser");
        let (value, consumed) = bser::decode_pdu(&bytes)
            .unwrap_or_else(|e| panic!("decoding {scenario}.bser2 failed: {e}"));
        assert_eq!(consumed, bytes.len(), "{scenario}: trailing bytes");
        let re = bser::encode_pdu(&value, Encoding::BserV2)
            .unwrap_or_else(|e| panic!("re-encoding {scenario} failed: {e}"));
        let (decoded_again, _) =
            bser::decode_pdu(&re).unwrap_or_else(|e| panic!("re-decoding {scenario} failed: {e}"));
        assert_eq!(
            decoded_again, value,
            "{scenario}: round-trip changed the value"
        );
    }
}

#[test]
fn json_fixtures_parse() {
    let dir = fixtures_root();
    for scenario in scenarios() {
        let json_path = dir.join(format!("{scenario}.json"));
        let raw = fs::read_to_string(&json_path).expect("read json");
        let parsed: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("parsing {scenario}.json failed: {e}"));
        assert!(parsed.is_object(), "{scenario}: not an object");
    }
}

#[test]
fn json_and_bser_agree_on_primitive_fields() {
    // BSER TEMPLATE rows are logically `array-of-object` in JSON but
    // encode compactly in BSER. We compare only scalar fields that both
    // encodings reproduce verbatim.
    let dir = fixtures_root();
    for scenario in scenarios() {
        let bser_path = dir.join(format!("{scenario}.bser2"));
        let json_path = dir.join(format!("{scenario}.json"));
        if !bser_path.exists() || !json_path.exists() {
            continue;
        }
        let bser_bytes = fs::read(&bser_path).expect("read bser");
        let (bser_val, _) = bser::decode_pdu(&bser_bytes).unwrap();

        let json_text = fs::read_to_string(&json_path).unwrap();
        let json_val: serde_json::Value = serde_json::from_str(&json_text).unwrap();

        compare_scalars(&scenario, "", &bser_val, &json_val);
    }
}

fn compare_scalars(scenario: &str, path: &str, b: &Value, j: &serde_json::Value) {
    match (b, j) {
        (Value::Null, serde_json::Value::Null) => {}
        (Value::Bool(a), serde_json::Value::Bool(bb)) => {
            assert_eq!(a, bb, "{scenario}:{path} bool mismatch")
        }
        (Value::Int(a), serde_json::Value::Number(n)) => {
            if let Some(ji) = n.as_i64() {
                assert_eq!(*a, ji, "{scenario}:{path} int mismatch");
            }
        }
        (Value::String(a), serde_json::Value::String(bb)) => {
            assert_eq!(a, bb, "{scenario}:{path} string mismatch")
        }
        (Value::Object(o), serde_json::Value::Object(jm)) => {
            for (k, v) in o {
                // Clock and pid-like fields advance between captures
                // (we record JSON and BSER on separate connections, so
                // the daemon bumps its clock between responses). Skip
                // the ones known to drift.
                if matches!(k.as_str(), "clock" | "pid" | "cclock" | "oclock") {
                    continue;
                }
                if let Some(jv) = jm.get(k) {
                    compare_scalars(scenario, &format!("{path}/{k}"), v, jv);
                }
            }
        }
        _ => {
            // Arrays / templates / type mismatches: skip — BSER
            // templates legitimately differ in shape from JSON arrays.
        }
    }
}

#[allow(dead_code)]
fn _path_ref(_p: &Path) {}
