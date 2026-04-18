use indexmap::IndexMap;

/// Shared, codec-agnostic value tree.
///
/// Watchman's JSON and BSER encodings both decode to the same logical
/// shape. We mirror that with a single enum so command handlers never
/// branch on the encoding that produced their input.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    Object(IndexMap<String, Value>),
    /// BSER template: a compact representation of an array of objects
    /// that share a field layout. Kept distinct so encoders can round-trip
    /// it back onto the wire.
    Template {
        keys: Vec<String>,
        rows: Vec<Vec<Value>>,
    },
}

impl Value {
    pub fn object() -> Self {
        Value::Object(IndexMap::new())
    }

    pub fn as_object(&self) -> Option<&IndexMap<String, Value>> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut IndexMap<String, Value>> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

/// Lightweight borrowed view used by serializers that want to avoid cloning.
#[derive(Debug, Clone, Copy)]
pub enum ValueRef<'a> {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Str(&'a str),
    Bytes(&'a [u8]),
    Array(&'a [Value]),
    Object(&'a IndexMap<String, Value>),
    Template {
        keys: &'a [String],
        rows: &'a [Vec<Value>],
    },
}

impl<'a> From<&'a Value> for ValueRef<'a> {
    fn from(v: &'a Value) -> Self {
        match v {
            Value::Null => ValueRef::Null,
            Value::Bool(b) => ValueRef::Bool(*b),
            Value::Int(i) => ValueRef::Int(*i),
            Value::Real(f) => ValueRef::Real(*f),
            Value::String(s) => ValueRef::Str(s.as_str()),
            Value::Bytes(b) => ValueRef::Bytes(b.as_slice()),
            Value::Array(a) => ValueRef::Array(a.as_slice()),
            Value::Object(o) => ValueRef::Object(o),
            Value::Template { keys, rows } => ValueRef::Template {
                keys: keys.as_slice(),
                rows: rows.as_slice(),
            },
        }
    }
}
