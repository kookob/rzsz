use std::io::{self, Read};
use std::os::unix::io::{AsFd, BorrowedFd};

use nix::poll::{poll, PollFd, PollFlags, PollTimeout};

/// Buffered reader for modem I/O with poll-based timeouts.
/// Replaces the C alarm()/SIGALRM approach with safe, deterministic polling.
pub struct ModemReader<R: Read + AsFd> {
    inner: R,
    buffer: Vec<u8>,
    pos: usize,
    len: usize,
    pushback: Option<u8>,
}

impl<R: Read + AsFd> ModemReader<R> {
    pub fn new(inner: R, buffer_size: usize) -> Self {
        Self {
            inner,
            buffer: vec![0u8; buffer_size],
            pos: 0,
            len: 0,
            pushback: None,
        }
    }

    /// Read a single byte with timeout (in tenths of seconds).
    /// Returns Ok(byte) or Err on timeout/IO error.
    /// Push a byte back into the reader (1-byte lookahead).
    pub fn unread_byte(&mut self, b: u8) {
        self.pushback = Some(b);
    }

    pub fn read_byte(&mut self, timeout_tenths: u32) -> io::Result<u8> {
        if let Some(b) = self.pushback.take() {
            return Ok(b);
        }
        if self.pos < self.len {
            let b = self.buffer[self.pos];
            self.pos += 1;
            return Ok(b);
        }

        // Buffer exhausted — poll for data, then refill
        let fd: BorrowedFd<'_> = self.inner.as_fd();
        let timeout_ms = (timeout_tenths as u16).saturating_mul(100);
        let timeout_ms = if timeout_ms < 100 { 100 } else { timeout_ms };

        let mut fds = [PollFd::new(fd, PollFlags::POLLIN)];
        let n = poll(&mut fds, PollTimeout::from(timeout_ms))
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "read timeout"));
        }

        let bytes_read = self.inner.read(&mut self.buffer)?;
        if bytes_read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed",
            ));
        }

        self.pos = 1;
        self.len = bytes_read;
        Ok(self.buffer[0])
    }

    /// Discard any buffered data.
    pub fn purge(&mut self) {
        self.pos = 0;
        self.len = 0;
    }
}
