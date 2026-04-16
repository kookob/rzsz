use std::io::{self, Read, Write};
use std::os::unix::io::AsFd;

use super::crc::{update_crc16, update_crc32, CRC32_MAGIC};
use super::escape::EscapeTable;
use super::frame::*;
use crate::serial::reader::ModemReader;

/// Protocol error types — exhaustive, no silent failures.
#[derive(Debug)]
pub enum ZError {
    CrcMismatch { expected: u32, got: u32 },
    Timeout,
    FrameError(String),
    Cancelled,
    Io(io::Error),
    InvalidFrame(u8),
    TooManyErrors,
    GarbageCount(usize),
}

impl From<io::Error> for ZError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::TimedOut {
            ZError::Timeout
        } else {
            ZError::Io(e)
        }
    }
}

impl std::fmt::Display for ZError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZError::CrcMismatch { expected, got } => {
                write!(f, "CRC mismatch: expected {expected:#x}, got {got:#x}")
            }
            ZError::Timeout => write!(f, "timeout"),
            ZError::FrameError(msg) => write!(f, "frame error: {msg}"),
            ZError::Cancelled => write!(f, "transfer cancelled"),
            ZError::Io(e) => write!(f, "I/O error: {e}"),
            ZError::InvalidFrame(b) => write!(f, "invalid frame type: {b:#x}"),
            ZError::TooManyErrors => write!(f, "too many errors"),
            ZError::GarbageCount(n) => write!(f, "too much garbage ({n} bytes)"),
        }
    }
}

impl std::error::Error for ZError {}

/// Explicit session state — compiler enforces exhaustive handling.
#[derive(Debug)]
pub enum SessionState {
    Init,
    Handshake { retries: u8 },
    FileHeader { filename: String, size: u64 },
    DataTransfer { offset: u64, block_size: usize },
    Eof,
    Fin,
}

/// Received header result.
pub struct ReceivedHeader {
    pub frame_type: FrameType,
    pub encoding: FrameEncoding,
    pub hdr: [u8; 4],
}

/// File metadata for transfer.
pub struct FileInfo {
    pub name: String,
    pub size: u64,
    pub mod_time: u64,
    pub mode: u32,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub bytes_skipped: u64,
}

/// ZModem session — all state is here, no globals.
pub struct Session {
    pub state: SessionState,
    pub encoder: FrameEncoder,
    pub escape_table: EscapeTable,
    pub rx_timeout_tenths: u32,
    pub errors: u32,
    pub max_retries: u32,
    pub max_block_size: usize,
    pub attn: Vec<u8>,
    pub escape_all_ctrl: bool,
    pub rx_window: usize,
}

impl Session {
    pub fn new() -> Self {
        Self {
            state: SessionState::Init,
            encoder: FrameEncoder::new(),
            escape_table: EscapeTable::new(false, false),
            rx_timeout_tenths: 100,
            errors: 0,
            max_retries: 10,
            max_block_size: 1024,
            attn: Vec::new(),
            escape_all_ctrl: false,
            rx_window: 1400,
        }
    }

    /// Read a byte from modem, filtering XON/XOFF and optionally control chars.
    /// Equivalent to noxrd7() in zm.c.
    fn read_filtered<R: Read + AsFd>(
        &self,
        reader: &mut ModemReader<R>,
    ) -> Result<u8, ZError> {
        loop {
            let c = reader.read_byte(self.rx_timeout_tenths)?;
            let c7 = c & 0x7f;
            match c7 {
                XON | XOFF => continue,
                _ if self.escape_all_ctrl && (c7 & 0x60) == 0 => continue,
                b'\r' | b'\n' | ZDLE => return Ok(c7),
                _ => return Ok(c7),
            }
        }
    }

    /// Decode a hex digit pair. Equivalent to zgethex() in zm.c.
    fn read_hex<R: Read + AsFd>(
        &self,
        reader: &mut ModemReader<R>,
    ) -> Result<u8, ZError> {
        let c = self.read_filtered(reader)?;
        let mut n = c.wrapping_sub(b'0');
        if n > 9 {
            n = n.wrapping_sub(b'a' - b':');
        }
        if n > 15 {
            return Err(ZError::FrameError("invalid hex digit".into()));
        }

        let c = self.read_filtered(reader)?;
        let mut low = c.wrapping_sub(b'0');
        if low > 9 {
            low = low.wrapping_sub(b'a' - b':');
        }
        if low > 15 {
            return Err(ZError::FrameError("invalid hex digit".into()));
        }

        Ok((n << 4) | low)
    }

    /// Read a ZDLE-escaped byte. Equivalent to zdlread() in zm.c.
    /// Uses iterative loop instead of recursion to avoid stack overflow on noise.
    fn read_escaped<R: Read + AsFd>(
        &self,
        reader: &mut ModemReader<R>,
    ) -> Result<u16, ZError> {
        loop {
            let c = reader.read_byte(self.rx_timeout_tenths)?;
            // Quick check for non-control characters
            if c & 0x60 != 0 {
                return Ok(c as u16);
            }

            // Handle special cases
            match c {
                ZDLE => {}
                XON | XOFF | 0x91 | 0x93 => continue, // Filter flow control
                _ if self.escape_all_ctrl && (c & 0x60) == 0 => continue,
                _ => return Ok(c as u16),
            }

            // After ZDLE — read the escaped byte
            loop {
                let c = reader.read_byte(self.rx_timeout_tenths)?;
                match c {
                    // CAN*5 abort detection
                    0x18 => {
                        let c2 = reader.read_byte(self.rx_timeout_tenths)?;
                        if c2 == 0x18 {
                            let c3 = reader.read_byte(self.rx_timeout_tenths)?;
                            if c3 == 0x18 {
                                let c4 = reader.read_byte(self.rx_timeout_tenths)?;
                                if c4 == 0x18 {
                                    return Err(ZError::Cancelled);
                                }
                            }
                        }
                        return Err(ZError::FrameError("partial CAN sequence".into()));
                    }
                    ZCRCE | ZCRCG | ZCRCQ | ZCRCW => {
                        return Ok(c as u16 | 0x100);
                    }
                    ZRUB0 => return Ok(0x7f),
                    ZRUB1 => return Ok(0xff),
                    XON | XOFF | 0x91 | 0x93 => continue, // Filter and retry
                    _ if self.escape_all_ctrl && (c & 0x60) == 0 => continue,
                    _ if (c & 0x60) == 0x40 => return Ok((c ^ 0x40) as u16),
                    _ => return Err(ZError::FrameError(format!("bad ZDLE sequence: {c:#x}"))),
                }
            }
        }
    }

    /// Receive a binary header with 16-bit CRC.
    fn receive_bin16_header<R: Read + AsFd>(
        &self,
        reader: &mut ModemReader<R>,
    ) -> Result<ReceivedHeader, ZError> {
        let type_val = self.read_escaped(reader)? as u8;
        let mut crc: u16 = update_crc16(0, type_val);
        let mut hdr = [0u8; 4];

        for b in hdr.iter_mut() {
            let val = self.read_escaped(reader)? as u8;
            *b = val;
            crc = update_crc16(crc, val);
        }

        let crc_hi = self.read_escaped(reader)? as u8;
        let crc_lo = self.read_escaped(reader)? as u8;
        crc = update_crc16(crc, crc_hi);
        crc = update_crc16(crc, crc_lo);

        if crc != 0 {
            return Err(ZError::CrcMismatch {
                expected: 0,
                got: crc as u32,
            });
        }

        let frame_type = FrameType::from_u8(type_val)
            .ok_or(ZError::InvalidFrame(type_val))?;

        Ok(ReceivedHeader {
            frame_type,
            encoding: FrameEncoding::Bin16,
            hdr,
        })
    }

    /// Receive a binary header with 32-bit CRC.
    fn receive_bin32_header<R: Read + AsFd>(
        &self,
        reader: &mut ModemReader<R>,
    ) -> Result<ReceivedHeader, ZError> {
        let type_val = self.read_escaped(reader)? as u8;
        let mut crc: u32 = update_crc32(0xFFFF_FFFF, type_val);
        let mut hdr = [0u8; 4];

        for b in hdr.iter_mut() {
            let val = self.read_escaped(reader)? as u8;
            *b = val;
            crc = update_crc32(crc, val);
        }

        for _ in 0..4 {
            let val = self.read_escaped(reader)? as u8;
            crc = update_crc32(crc, val);
        }

        if crc != CRC32_MAGIC {
            return Err(ZError::CrcMismatch {
                expected: CRC32_MAGIC,
                got: crc,
            });
        }

        let frame_type = FrameType::from_u8(type_val)
            .ok_or(ZError::InvalidFrame(type_val))?;

        Ok(ReceivedHeader {
            frame_type,
            encoding: FrameEncoding::Bin32,
            hdr,
        })
    }

    /// Receive a hex header.
    fn receive_hex_header<R: Read + AsFd>(
        &self,
        reader: &mut ModemReader<R>,
    ) -> Result<ReceivedHeader, ZError> {
        let type_val = self.read_hex(reader)?;
        let mut crc: u16 = update_crc16(0, type_val);
        let mut hdr = [0u8; 4];

        for b in hdr.iter_mut() {
            let val = self.read_hex(reader)?;
            *b = val;
            crc = update_crc16(crc, val);
        }

        let crc_hi = self.read_hex(reader)?;
        let crc_lo = self.read_hex(reader)?;
        crc = update_crc16(crc, crc_hi);
        crc = update_crc16(crc, crc_lo);

        if crc != 0 {
            return Err(ZError::CrcMismatch {
                expected: 0,
                got: crc as u32,
            });
        }

        // Read and discard trailing CR/LF and possible XON
        let c = reader.read_byte(self.rx_timeout_tenths)?;
        if c == b'\r' {
            let _ = reader.read_byte(self.rx_timeout_tenths); // LF
        }
        // Possible XON — push back if it's not XON (belongs to next frame)
        if let Ok(b) = reader.read_byte(1) {
            if b != XON {
                reader.unread_byte(b);
            }
        }

        let frame_type = FrameType::from_u8(type_val)
            .ok_or(ZError::InvalidFrame(type_val))?;

        Ok(ReceivedHeader {
            frame_type,
            encoding: FrameEncoding::Hex,
            hdr,
        })
    }

    /// Receive any header — auto-detects encoding.
    /// Equivalent to zgethdr() in zm.c.
    pub fn receive_header<R: Read + AsFd>(
        &self,
        reader: &mut ModemReader<R>,
    ) -> Result<ReceivedHeader, ZError> {
        let mut garbage_count: usize = 0;

        loop {
            let c = reader.read_byte(self.rx_timeout_tenths)?;

            match c {
                ZPAD => {}
                _ => {
                    garbage_count += 1;
                    if garbage_count > self.rx_window {
                        return Err(ZError::GarbageCount(garbage_count));
                    }
                    continue;
                }
            }

            // Got ZPAD — look for second ZPAD or ZDLE
            let c = reader.read_byte(self.rx_timeout_tenths)?;
            if c == ZPAD {
                // Second ZPAD — next must be ZDLE
                let c = reader.read_byte(self.rx_timeout_tenths)?;
                if c != ZDLE {
                    garbage_count += 3;
                    continue;
                }
            } else if c != ZDLE {
                garbage_count += 2;
                continue;
            }

            // Got ZPAD ZDLE (or ZPAD ZPAD ZDLE) — read encoding byte
            let c = reader.read_byte(self.rx_timeout_tenths)?;
            match c {
                ZBIN => return self.receive_bin16_header(reader),
                ZBIN32 => return self.receive_bin32_header(reader),
                ZHEX => return self.receive_hex_header(reader),
                _ => {
                    garbage_count += 1;
                    continue;
                }
            }
        }
    }

    /// Receive data with 16-bit CRC.
    pub fn receive_data16<R: Read + AsFd>(
        &self,
        reader: &mut ModemReader<R>,
        buf: &mut Vec<u8>,
        max_len: usize,
    ) -> Result<FrameEnd, ZError> {
        let mut crc: u16 = 0;
        buf.clear();

        loop {
            let val = self.read_escaped(reader)?;
            if val & 0x100 != 0 {
                // Frame end marker
                let end_byte = (val & 0xff) as u8;
                crc = update_crc16(crc, end_byte);

                let crc_hi = self.read_escaped(reader)? as u8;
                let crc_lo = self.read_escaped(reader)? as u8;
                crc = update_crc16(crc, crc_hi);
                crc = update_crc16(crc, crc_lo);

                if crc != 0 {
                    return Err(ZError::CrcMismatch {
                        expected: 0,
                        got: crc as u32,
                    });
                }

                return FrameEnd::from_u8(end_byte).ok_or_else(|| {
                    ZError::FrameError(format!("invalid frame end: {end_byte:#x}"))
                });
            }

            if buf.len() >= max_len {
                return Err(ZError::FrameError("data exceeds max length".into()));
            }
            let byte = val as u8;
            buf.push(byte);
            crc = update_crc16(crc, byte);
        }
    }

    /// Receive data with 32-bit CRC.
    pub fn receive_data32<R: Read + AsFd>(
        &self,
        reader: &mut ModemReader<R>,
        buf: &mut Vec<u8>,
        max_len: usize,
    ) -> Result<FrameEnd, ZError> {
        let mut crc: u32 = 0xFFFF_FFFF;
        buf.clear();

        loop {
            let val = self.read_escaped(reader)?;
            if val & 0x100 != 0 {
                let end_byte = (val & 0xff) as u8;
                crc = update_crc32(crc, end_byte);

                for _ in 0..4 {
                    let b = self.read_escaped(reader)? as u8;
                    crc = update_crc32(crc, b);
                }

                if crc != CRC32_MAGIC {
                    return Err(ZError::CrcMismatch {
                        expected: CRC32_MAGIC,
                        got: crc,
                    });
                }

                return FrameEnd::from_u8(end_byte).ok_or_else(|| {
                    ZError::FrameError(format!("invalid frame end: {end_byte:#x}"))
                });
            }

            if buf.len() >= max_len {
                return Err(ZError::FrameError("data exceeds max length".into()));
            }
            let byte = val as u8;
            buf.push(byte);
            crc = update_crc32(crc, byte);
        }
    }

    /// Send a hex header with position.
    pub fn send_pos_header<W: Write>(
        &mut self,
        frame_type: FrameType,
        pos: u64,
        out: &mut W,
    ) -> io::Result<()> {
        let hdr = store_position(pos);
        self.encoder.send_hex_header(frame_type, &hdr, out)
    }

    /// Send a binary header with position.
    pub fn send_bin_pos_header<W: Write>(
        &mut self,
        frame_type: FrameType,
        pos: u64,
        out: &mut W,
    ) -> io::Result<()> {
        let hdr = store_position(pos);
        self.encoder
            .send_binary_header(frame_type, &hdr, 0, &self.escape_table, out)
    }

    /// Send data frame.
    pub fn send_data<W: Write>(
        &mut self,
        data: &[u8],
        frame_end: FrameEnd,
        out: &mut W,
    ) -> io::Result<()> {
        self.encoder
            .send_data(data, frame_end, &self.escape_table, out)
    }
}
