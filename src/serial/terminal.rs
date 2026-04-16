use nix::sys::termios::{self, Termios, SetArg};
use std::os::unix::io::{BorrowedFd, RawFd};

/// RAII guard for terminal mode — Drop automatically restores original settings.
/// Guarantees terminal is restored on any exit path: normal return, `?`, panic.
/// Uses raw fd (not OwnedFd) to avoid closing stdin/stdout on drop.
pub struct TerminalGuard {
    fd: RawFd,
    original: Termios,
}

impl TerminalGuard {
    /// Save current terminal settings and return a guard.
    /// Returns Err if the fd is not a terminal (e.g. a pipe).
    pub fn new(fd: RawFd) -> nix::Result<Self> {
        let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
        let original = termios::tcgetattr(&borrowed)?;
        Ok(Self { fd, original })
    }

    fn borrowed_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(self.fd) }
    }

    /// Set terminal to raw mode for protocol data transfer.
    pub fn set_raw(&self) -> nix::Result<()> {
        let mut raw = self.original.clone();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(&self.borrowed_fd(), SetArg::TCSADRAIN, &raw)
    }

    /// Set raw mode with flow control enabled.
    pub fn set_raw_with_flow_control(&self) -> nix::Result<()> {
        let mut raw = self.original.clone();
        termios::cfmakeraw(&mut raw);
        raw.input_flags |= nix::sys::termios::InputFlags::IXON
            | nix::sys::termios::InputFlags::IXOFF;
        termios::tcsetattr(&self.borrowed_fd(), SetArg::TCSADRAIN, &raw)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = termios::tcsetattr(&self.borrowed_fd(), SetArg::TCSADRAIN, &self.original);
    }
}
