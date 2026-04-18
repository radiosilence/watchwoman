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
    MtimeNs,
    Ctime,
    CtimeMs,
    CtimeNs,
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
            "mtime_ns" => Self::MtimeNs,
            "ctime" => Self::Ctime,
            "ctime_ms" => Self::CtimeMs,
            "ctime_ns" => Self::CtimeNs,
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
            Self::MtimeNs => "mtime_ns",
            Self::Ctime => "ctime",
            Self::CtimeMs => "ctime_ms",
            Self::CtimeNs => "ctime_ns",
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
    let mut out = IndexMap::with_capacity(fields.len());
    for f in fields {
        let v = match f {
            Field::Name => Value::String(rel.to_string_lossy().into_owned()),
            Field::Exists => Value::Bool(entry.exists),
            Field::Size => Value::Int(entry.size as i64),
            Field::Mode => Value::Int(entry.mode as i64),
            Field::Uid => Value::Int(0),
            Field::Gid => Value::Int(0),
            Field::Ino => Value::Int(entry.ino as i64),
            Field::Dev => Value::Int(entry.dev as i64),
            Field::Nlink => Value::Int(entry.nlink as i64),
            Field::Mtime => Value::Int(entry.mtime_ms / 1000),
            Field::MtimeMs => Value::Int(entry.mtime_ms),
            Field::MtimeNs => Value::Int(entry.mtime_ns as i64),
            Field::Ctime => Value::Int(entry.ctime_ms / 1000),
            Field::CtimeMs => Value::Int(entry.ctime_ms),
            Field::CtimeNs => Value::Int(entry.ctime_ns as i64),
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
