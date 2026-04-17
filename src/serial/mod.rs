pub mod reader;
pub mod terminal;

use std::io::{self, Write};

/// Protocol data writer (stdout) — newtype enforces separation at compile time.
pub struct ProtocolWriter(io::Stdout);

impl Default for ProtocolWriter {
    fn default() -> Self { Self::new() }
}

impl ProtocolWriter {
    pub fn new() -> Self {
        Self(io::stdout())
    }
}

impl Write for ProtocolWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// Status/progress writer (stderr) — type-level separation from protocol I/O.
pub struct StatusWriter(io::Stderr);

impl Default for StatusWriter {
    fn default() -> Self { Self::new() }
}

impl StatusWriter {
    pub fn new() -> Self {
        Self(io::stderr())
    }
}

impl Write for StatusWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.0.write(buf)?;
        self.0.flush()?;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
