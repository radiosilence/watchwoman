use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use watchwoman_protocol::{json, Value};

/// Minimal JSON-speaking client used by tests.
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Client {
    pub fn connect(sock: &Path) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(sock)
            .with_context(|| format!("connecting to {}", sock.display()))?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            reader,
            writer: stream,
        })
    }

    /// Send a command in the `[name, ...args]` watchman shape and return
    /// the response.
    pub fn send(&mut self, pdu: Value) -> anyhow::Result<Value> {
        json::encode_pdu(&mut self.writer, &pdu)?;
        self.writer.flush()?;
        let resp =
            json::read_pdu(&mut self.reader)?.context("connection closed before response")?;
        Ok(resp)
    }

    /// Build `[name, ...args]` for you.
    pub fn call<I>(&mut self, name: &str, args: I) -> anyhow::Result<Value>
    where
        I: IntoIterator<Item = Value>,
    {
        let mut v = vec![Value::String(name.into())];
        v.extend(args);
        self.send(Value::Array(v))
    }

    /// Read the next unilateral PDU (e.g. subscription notification).
    pub fn read_unilateral(&mut self) -> anyhow::Result<Option<Value>> {
        Ok(json::read_pdu(&mut self.reader)?)
    }
}

impl std::io::Read for Client {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.reader.read(buf)
    }
}

impl BufRead for Client {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.reader.fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        self.reader.consume(amt)
    }
}
