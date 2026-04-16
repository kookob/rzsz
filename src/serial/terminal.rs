use nix::sys::termios::{self, Termios, SetArg};
use std::os::unix::io::OwnedFd;
use std::os::unix::io::FromRawFd;

/// RAII guard for terminal mode — Drop automatically restores original settings.
/// Guarantees terminal is restored on any exit path: normal return, `?`, panic.
pub struct TerminalGuard {
    fd: OwnedFd,
    original: Termios,
}

impl TerminalGuard {
    /// Save current terminal settings and return a guard.
    /// # Safety
    /// The caller must ensure `raw_fd` is a valid, open file descriptor
    /// that will remain valid for the lifetime of this guard.
    pub unsafe fn from_raw_fd(raw_fd: i32) -> nix::Result<Self> {
        let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        let original = termios::tcgetattr(&fd)?;
        Ok(Self { fd, original })
    }

    /// Set terminal to raw mode for protocol data transfer.
    pub fn set_raw(&self) -> nix::Result<()> {
        let mut raw = self.original.clone();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(&self.fd, SetArg::TCSADRAIN, &raw)
    }

    /// Set raw mode with flow control enabled.
    pub fn set_raw_with_flow_control(&self) -> nix::Result<()> {
        let mut raw = self.original.clone();
        termios::cfmakeraw(&mut raw);
        raw.input_flags |= nix::sys::termios::InputFlags::IXON
            | nix::sys::termios::InputFlags::IXOFF;
        termios::tcsetattr(&self.fd, SetArg::TCSADRAIN, &raw)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = termios::tcsetattr(&self.fd, SetArg::TCSADRAIN, &self.original);
    }
}
