//! Serialise [`FileEntry`] fields to watchman's wire shape.

use std::path::Path;

use indexmap::IndexMap;
use watchwoman_protocol::Value;

use crate::commands::CommandError;
use crate::daemon::clock::Clock;
use crate::daemon::tree::FileEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Exists,
    Size,
    Mode,
    Uid,
    Gid,
    Ino,
    Dev,
    Nlink,
    Mtime,
    MtimeMs,
    MtimeUs,
    MtimeNs,
    MtimeF,
    Ctime,
    CtimeMs,
    CtimeUs,
    CtimeNs,
    CtimeF,
    Type,
    New,
    CClock,
    OClock,
    SymlinkTarget,
    ContentSha1Hex,
}

impl Field {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "name" => Self::Name,
            "exists" => Self::Exists,
            "size" => Self::Size,
            "mode" => Self::Mode,
            "uid" => Self::Uid,
            "gid" => Self::Gid,
            "ino" => Self::Ino,
            "dev" => Self::Dev,
            "nlink" => Self::Nlink,
            "mtime" => Self::Mtime,
            "mtime_ms" => Self::MtimeMs,
            "mtime_us" => Self::MtimeUs,
            "mtime_ns" => Self::MtimeNs,
            "mtime_f" => Self::MtimeF,
            "ctime" => Self::Ctime,
            "ctime_ms" => Self::CtimeMs,
            "ctime_us" => Self::CtimeUs,
            "ctime_ns" => Self::CtimeNs,
            "ctime_f" => Self::CtimeF,
            "type" => Self::Type,
            "new" => Self::New,
            "cclock" => Self::CClock,
            "oclock" => Self::OClock,
            "symlink_target" => Self::SymlinkTarget,
            "content.sha1hex" => Self::ContentSha1Hex,
            _ => return None,
        })
    }

    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Exists => "exists",
            Self::Size => "size",
            Self::Mode => "mode",
            Self::Uid => "uid",
            Self::Gid => "gid",
            Self::Ino => "ino",
            Self::Dev => "dev",
            Self::Nlink => "nlink",
            Self::Mtime => "mtime",
            Self::MtimeMs => "mtime_ms",
            Self::MtimeUs => "mtime_us",
            Self::MtimeNs => "mtime_ns",
            Self::MtimeF => "mtime_f",
            Self::Ctime => "ctime",
            Self::CtimeMs => "ctime_ms",
            Self::CtimeUs => "ctime_us",
            Self::CtimeNs => "ctime_ns",
            Self::CtimeF => "ctime_f",
            Self::Type => "type",
            Self::New => "new",
            Self::CClock => "cclock",
            Self::OClock => "oclock",
            Self::SymlinkTarget => "symlink_target",
            Self::ContentSha1Hex => "content.sha1hex",
        }
    }
}

pub fn default_fields() -> Vec<Field> {
    vec![
        Field::Name,
        Field::Exists,
        Field::New,
        Field::Size,
        Field::Mode,
        Field::Type,
    ]
}

pub fn parse_list(value: &Value) -> Result<Vec<Field>, CommandError> {
    let arr = value
        .as_array()
        .ok_or_else(|| CommandError::BadArgs("`fields` must be an array".into()))?;
    let mut out = Vec::with_capacity(arr.len());
    for v in arr {
        let name = v
            .as_str()
            .ok_or_else(|| CommandError::BadArgs("`fields` entries must be strings".into()))?;
        if let Some(f) = Field::parse(name) {
            out.push(f);
        } else {
            return Err(CommandError::BadArgs(format!("unknown field `{name}`")));
        }
    }
    Ok(out)
}

pub fn render_row(
    root_path: &Path,
    rel: &Path,
    entry: &FileEntry,
    fields: &[Field],
    clock: &Clock,
) -> Value {
    // Watchman's documented shortcut: if the caller asked for exactly
    // one field AND that field is `name`, return bare strings in
    // `files[]` instead of objects.  Jest, Metro, Sapling et al. all
    // rely on this — breaking it shows up as "files is not an array
    // of strings" from the client side.
    if fields.len() == 1 && fields[0] == Field::Name {
        return Value::String(rel.to_string_lossy().into_owned());
    }
    let mut out = IndexMap::with_capacity(fields.len());
    for f in fields {
        let v = match f {
            Field::Name => Value::String(rel.to_string_lossy().into_owned()),
            Field::Exists => Value::Bool(entry.exists),
            Field::Size => Value::Int(entry.size as i64),
            Field::Mode => Value::Int(entry.mode as i64),
            Field::Uid => Value::Int(entry.uid as i64),
            Field::Gid => Value::Int(entry.gid as i64),
            Field::Ino => Value::Int(entry.ino as i64),
            Field::Dev => Value::Int(entry.dev as i64),
            Field::Nlink => Value::Int(entry.nlink as i64),
            Field::Mtime => Value::Int(entry.mtime_ms / 1000),
            Field::MtimeMs => Value::Int(entry.mtime_ms),
            Field::MtimeUs => Value::Int((entry.mtime_ns / 1_000) as i64),
            Field::MtimeNs => Value::Int(entry.mtime_ns as i64),
            Field::MtimeF => Value::Real((entry.mtime_ns as f64) / 1_000_000_000.0),
            Field::Ctime => Value::Int(entry.ctime_ms / 1000),
            Field::CtimeMs => Value::Int(entry.ctime_ms),
            Field::CtimeUs => Value::Int((entry.ctime_ns / 1_000) as i64),
            Field::CtimeNs => Value::Int(entry.ctime_ns as i64),
            Field::CtimeF => Value::Real((entry.ctime_ns as f64) / 1_000_000_000.0),
            Field::Type => Value::String(entry.kind.as_wire_str().into()),
            Field::New => Value::Bool(entry.is_new),
            Field::CClock => Value::String(clock.encode(entry.cclock)),
            Field::OClock => Value::String(clock.encode(entry.oclock)),
            Field::SymlinkTarget => match &entry.symlink_target {
                Some(t) => Value::String(t.clone()),
                None => Value::Null,
            },
            Field::ContentSha1Hex => sha1_of(&root_path.join(rel)),
        };
        out.insert(f.wire_name().to_owned(), v);
    }
    // Single-field-`name` queries degenerate to strings in watchman; keep
    // the object shape so tests can uniformly introspect.
    Value::Object(out)
}

fn sha1_of(abs: &Path) -> Value {
    use sha1::Digest;
    let Ok(mut file) = std::fs::File::open(abs) else {
        return Value::Null;
    };
    let mut hasher = sha1::Sha1::new();
    let mut buf = [0u8; 8192];
    loop {
        match std::io::Read::read(&mut file, &mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => return Value::Null,
        }
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(40);
    for b in digest {
        hex.push_str(&format!("{b:02x}"));
    }
    Value::String(hex)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::tree::{FileEntry, FileKind};

    fn synthetic_entry(mtime_ns: i128, ctime_ns: i128) -> FileEntry {
        FileEntry {
            exists: true,
            kind: FileKind::File,
            size: 0,
            mode: 0o644,
            uid: 0,
            gid: 0,
            mtime_ns,
            ctime_ns,
            mtime_ms: (mtime_ns / 1_000_000) as i64,
            ctime_ms: (ctime_ns / 1_000_000) as i64,
            ino: 0,
            dev: 0,
            nlink: 1,
            symlink_target: None,
            cclock: 0,
            oclock: 0,
            is_new: false,
        }
    }

    #[test]
    fn us_and_fractional_time_fields_derive_from_ns() {
        // Pick an mtime with a non-zero sub-second component so the
        // us / fractional derivations aren't trivially right.
        let mtime_ns: i128 = 1_700_000_000_123_456_789;
        let ctime_ns: i128 = 1_600_000_000_987_654_321;
        let entry = synthetic_entry(mtime_ns, ctime_ns);
        let clock = Clock::new(0);
        let fields = [
            Field::MtimeUs,
            Field::MtimeF,
            Field::CtimeUs,
            Field::CtimeF,
            Field::MtimeNs,
        ];
        let row = render_row(Path::new("/tmp"), Path::new("x"), &entry, &fields, &clock);
        let obj = row.as_object().expect("object row");

        // Integer division — us is ns / 1_000 truncated.
        let mtime_us = obj.get("mtime_us").and_then(Value::as_i64).unwrap();
        assert_eq!(mtime_us, (mtime_ns / 1_000) as i64);
        let ctime_us = obj.get("ctime_us").and_then(Value::as_i64).unwrap();
        assert_eq!(ctime_us, (ctime_ns / 1_000) as i64);

        // Fractional seconds — ns as f64 divided by 1e9.
        let mtime_f = match obj.get("mtime_f") {
            Some(Value::Real(f)) => *f,
            other => panic!("expected Real for mtime_f, got {other:?}"),
        };
        assert!((mtime_f - (mtime_ns as f64) / 1_000_000_000.0).abs() < 1e-6);
        let ctime_f = match obj.get("ctime_f") {
            Some(Value::Real(f)) => *f,
            other => panic!("expected Real for ctime_f, got {other:?}"),
        };
        assert!((ctime_f - (ctime_ns as f64) / 1_000_000_000.0).abs() < 1e-6);
    }

    #[test]
    fn new_fields_parse_and_wire_round_trip() {
        for name in ["mtime_us", "mtime_f", "ctime_us", "ctime_f"] {
            let f = Field::parse(name).unwrap_or_else(|| panic!("parse {name}"));
            assert_eq!(f.wire_name(), name);
        }
    }
}
