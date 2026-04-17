use std::io::{self, Write};

use super::crc::{update_crc16, update_crc32};
use super::escape::EscapeTable;

/// ZModem frame types — exhaustive enum forces handling of all protocol states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameType {
    ZrqInit = 0,
    ZrInit = 1,
    ZsInit = 2,
    ZAck = 3,
    ZFile = 4,
    ZSkip = 5,
    ZNak = 6,
    ZAbort = 7,
    ZFin = 8,
    ZRpos = 9,
    ZData = 10,
    ZEof = 11,
    ZFerr = 12,
    ZCrc = 13,
    ZChallenge = 14,
    ZCompl = 15,
    ZCan = 16,
    ZFreeCnt = 17,
    ZCommand = 18,
    ZStderr = 19,
}

impl FrameType {
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0 => Some(Self::ZrqInit),
            1 => Some(Self::ZrInit),
            2 => Some(Self::ZsInit),
            3 => Some(Self::ZAck),
            4 => Some(Self::ZFile),
            5 => Some(Self::ZSkip),
            6 => Some(Self::ZNak),
            7 => Some(Self::ZAbort),
            8 => Some(Self::ZFin),
            9 => Some(Self::ZRpos),
            10 => Some(Self::ZData),
            11 => Some(Self::ZEof),
            12 => Some(Self::ZFerr),
            13 => Some(Self::ZCrc),
            14 => Some(Self::ZChallenge),
            15 => Some(Self::ZCompl),
            16 => Some(Self::ZCan),
            17 => Some(Self::ZFreeCnt),
            18 => Some(Self::ZCommand),
            19 => Some(Self::ZStderr),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::ZrqInit => "ZRQINIT",
            Self::ZrInit => "ZRINIT",
            Self::ZsInit => "ZSINIT",
            Self::ZAck => "ZACK",
            Self::ZFile => "ZFILE",
            Self::ZSkip => "ZSKIP",
            Self::ZNak => "ZNAK",
            Self::ZAbort => "ZABORT",
            Self::ZFin => "ZFIN",
            Self::ZRpos => "ZRPOS",
            Self::ZData => "ZDATA",
            Self::ZEof => "ZEOF",
            Self::ZFerr => "ZFERR",
            Self::ZCrc => "ZCRC",
            Self::ZChallenge => "ZCHALLENGE",
            Self::ZCompl => "ZCOMPL",
            Self::ZCan => "ZCAN",
            Self::ZFreeCnt => "ZFREECNT",
            Self::ZCommand => "ZCOMMAND",
            Self::ZStderr => "ZSTDERR",
        }
    }
}

/// Frame encoding types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameEncoding {
    Bin16,
    Bin32,
    Hex,
}

/// Data frame ending codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameEnd {
    CrcE, // ZCRCE: end of frame, header follows
    CrcG, // ZCRCG: continue, no ACK needed
    CrcQ, // ZCRCQ: continue, ACK required
    CrcW, // ZCRCW: end of frame, ACK required
}

impl FrameEnd {
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            ZCRCE => Some(Self::CrcE),
            ZCRCG => Some(Self::CrcG),
            ZCRCQ => Some(Self::CrcQ),
            ZCRCW => Some(Self::CrcW),
            _ => None,
        }
    }

    pub fn as_u8(&self) -> u8 {
        match self {
            Self::CrcE => ZCRCE,
            Self::CrcG => ZCRCG,
            Self::CrcQ => ZCRCQ,
            Self::CrcW => ZCRCW,
        }
    }
}

// Protocol constants
pub const ZPAD: u8 = b'*';
pub const ZDLE: u8 = 0x18;
pub const ZBIN: u8 = b'A';
pub const ZHEX: u8 = b'B';
pub const ZBIN32: u8 = b'C';

pub const ZCRCE: u8 = b'h';
pub const ZCRCG: u8 = b'i';
pub const ZCRCQ: u8 = b'j';
pub const ZCRCW: u8 = b'k';

pub const ZRUB0: u8 = b'l';
pub const ZRUB1: u8 = b'm';

pub const XON: u8 = 0x11;
pub const XOFF: u8 = 0x13;

// Receiver capability flags
pub const CANFDX: u8 = 0x01;
pub const CANOVIO: u8 = 0x02;
pub const CANBRK: u8 = 0x04;
pub const CANFC32: u8 = 0x20;
pub const ESCCTL: u8 = 0x40;

pub const ZATTNLEN: usize = 32;

// File management options (ZF1)
pub const ZF1_ZMNEW: u8 = 1;
pub const ZF1_ZMCRC: u8 = 2;
pub const ZF1_ZMAPND: u8 = 3;
pub const ZF1_ZMCLOB: u8 = 4;
pub const ZF1_ZMDIFF: u8 = 5;
pub const ZF1_ZMPROT: u8 = 6;
pub const ZF1_ZMCHNG: u8 = 7;

/// Hex digit lookup
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Encode a byte as two hex ASCII characters.
#[inline]
fn put_hex(val: u8, out: &mut [u8]) {
    out[0] = HEX_DIGITS[(val >> 4) as usize];
    out[1] = HEX_DIGITS[(val & 0x0f) as usize];
}

/// Frame encoder — builds and writes ZModem headers and data frames.
pub struct FrameEncoder {
    pub use_crc32: bool,
    last_sent: u8,
}

impl Default for FrameEncoder {
    fn default() -> Self { Self::new() }
}

impl FrameEncoder {
    pub fn new() -> Self {
        Self {
            use_crc32: false,
            last_sent: 0,
        }
    }

    /// Send a single byte with ZDLE escape encoding.
    fn zsendline<W: Write>(
        &mut self,
        c: u8,
        escape: &EscapeTable,
        out: &mut W,
    ) -> io::Result<()> {
        let mut buf = [0u8; 2];
        let (len, last) = escape.encode(c, self.last_sent, &mut buf);
        out.write_all(&buf[..len])?;
        self.last_sent = last;
        Ok(())
    }

    /// Send a raw byte without escaping.
    #[inline]
    fn xsendline<W: Write>(&mut self, c: u8, out: &mut W) -> io::Result<()> {
        out.write_all(&[c])?;
        self.last_sent = c;
        Ok(())
    }

    /// Send binary header with 16-bit CRC (ZBIN frame).
    pub fn send_bin16_header<W: Write>(
        &mut self,
        frame_type: FrameType,
        hdr: &[u8; 4],
        null_prefix: usize,
        escape: &EscapeTable,
        out: &mut W,
    ) -> io::Result<()> {
        let type_byte = frame_type as u8;

        // ZDATA gets null prefix
        if frame_type == FrameType::ZData {
            for _ in 0..null_prefix {
                self.xsendline(0, out)?;
            }
        }

        self.xsendline(ZPAD, out)?;
        self.xsendline(ZDLE, out)?;
        self.xsendline(ZBIN, out)?;
        self.zsendline(type_byte, escape, out)?;

        let mut crc: u16 = update_crc16(0, type_byte);
        for &b in hdr.iter() {
            self.zsendline(b, escape, out)?;
            crc = update_crc16(crc, b);
        }
        // Finalize CRC
        crc = update_crc16(update_crc16(crc, 0), 0);
        self.zsendline((crc >> 8) as u8, escape, out)?;
        self.zsendline(crc as u8, escape, out)?;

        if frame_type != FrameType::ZData {
            out.flush()?;
        }
        Ok(())
    }

    /// Send binary header with 32-bit CRC (ZBIN32 frame).
    pub fn send_bin32_header<W: Write>(
        &mut self,
        frame_type: FrameType,
        hdr: &[u8; 4],
        null_prefix: usize,
        escape: &EscapeTable,
        out: &mut W,
    ) -> io::Result<()> {
        let type_byte = frame_type as u8;

        if frame_type == FrameType::ZData {
            for _ in 0..null_prefix {
                self.xsendline(0, out)?;
            }
        }

        self.xsendline(ZPAD, out)?;
        self.xsendline(ZDLE, out)?;
        self.xsendline(ZBIN32, out)?;
        self.zsendline(type_byte, escape, out)?;

        let mut crc: u32 = update_crc32(0xFFFF_FFFF, type_byte);
        for &b in hdr.iter() {
            crc = update_crc32(crc, b);
            self.zsendline(b, escape, out)?;
        }
        crc = !crc;
        for _ in 0..4 {
            self.zsendline(crc as u8, escape, out)?;
            crc >>= 8;
        }

        if frame_type != FrameType::ZData {
            out.flush()?;
        }
        Ok(())
    }

    /// Send header — dispatches to bin16 or bin32 based on use_crc32.
    pub fn send_binary_header<W: Write>(
        &mut self,
        frame_type: FrameType,
        hdr: &[u8; 4],
        null_prefix: usize,
        escape: &EscapeTable,
        out: &mut W,
    ) -> io::Result<()> {
        if self.use_crc32 {
            self.send_bin32_header(frame_type, hdr, null_prefix, escape, out)
        } else {
            self.send_bin16_header(frame_type, hdr, null_prefix, escape, out)
        }
    }

    /// Send hex header (ZHEX frame) — always uses CRC-16.
    pub fn send_hex_header<W: Write>(
        &mut self,
        frame_type: FrameType,
        hdr: &[u8; 4],
        out: &mut W,
    ) -> io::Result<()> {
        let type_byte = frame_type as u8 & 0x7f;
        let mut buf = [0u8; 30];
        buf[0] = ZPAD;
        buf[1] = ZPAD;
        buf[2] = ZDLE;
        buf[3] = ZHEX;
        put_hex(type_byte, &mut buf[4..6]);
        let mut len = 6;

        let mut crc: u16 = update_crc16(0, type_byte);
        for &b in hdr.iter() {
            put_hex(b, &mut buf[len..len + 2]);
            len += 2;
            crc = update_crc16(crc, b);
        }
        crc = update_crc16(update_crc16(crc, 0), 0);
        put_hex((crc >> 8) as u8, &mut buf[len..len + 2]);
        put_hex(crc as u8, &mut buf[len + 2..len + 4]);
        len += 4;

        // CR LF
        buf[len] = 0o15; // CR
        len += 1;
        buf[len] = 0o212; // LF with high bit
        len += 1;

        // XON to uncork (except for ZFIN and ZACK)
        if frame_type != FrameType::ZFin && frame_type != FrameType::ZAck {
            buf[len] = XON;
            len += 1;
        }

        out.write_all(&buf[..len])?;
        out.flush()?;
        Ok(())
    }

    /// Encode one byte with ZDLE escape into a caller-provided buffer.
    /// Returns the number of bytes written. Avoids per-byte syscalls.
    #[inline]
    fn encode_into(
        buf: &mut Vec<u8>,
        byte: u8,
        last_sent: &mut u8,
        escape: &EscapeTable,
    ) {
        let mut tmp = [0u8; 2];
        let (len, last) = escape.encode(byte, *last_sent, &mut tmp);
        buf.extend_from_slice(&tmp[..len]);
        *last_sent = last;
    }

    /// Send data with 16-bit CRC — builds the full escaped frame in memory
    /// and writes it in one call to avoid per-byte syscalls.
    pub fn send_data16<W: Write>(
        &mut self,
        data: &[u8],
        frame_end: FrameEnd,
        escape: &EscapeTable,
        out: &mut W,
    ) -> io::Result<()> {
        // Worst case: every byte escaped (×2) + frame terminator + CRC + XON
        let mut buf: Vec<u8> = Vec::with_capacity(data.len() * 2 + 8);
        let mut crc: u16 = 0;
        let mut last = self.last_sent;

        for &b in data {
            Self::encode_into(&mut buf, b, &mut last, escape);
            crc = update_crc16(crc, b);
        }

        let end_byte = frame_end.as_u8();
        // ZDLE + end_byte go raw (no escape) — matches xsendline behavior
        buf.push(ZDLE);
        buf.push(end_byte);
        last = end_byte;
        crc = update_crc16(crc, end_byte);
        crc = update_crc16(update_crc16(crc, 0), 0);

        Self::encode_into(&mut buf, (crc >> 8) as u8, &mut last, escape);
        Self::encode_into(&mut buf, crc as u8, &mut last, escape);

        if frame_end == FrameEnd::CrcW {
            buf.push(XON);
            last = XON;
        }

        out.write_all(&buf)?;
        self.last_sent = last;

        if frame_end == FrameEnd::CrcW {
            out.flush()?;
        }
        Ok(())
    }

    /// Send data with 32-bit CRC — batched, same strategy as `send_data16`.
    pub fn send_data32<W: Write>(
        &mut self,
        data: &[u8],
        frame_end: FrameEnd,
        escape: &EscapeTable,
        out: &mut W,
    ) -> io::Result<()> {
        let mut buf: Vec<u8> = Vec::with_capacity(data.len() * 2 + 12);
        let mut crc: u32 = 0xFFFF_FFFF;
        let mut last = self.last_sent;

        for &b in data {
            Self::encode_into(&mut buf, b, &mut last, escape);
            crc = update_crc32(crc, b);
        }

        let end_byte = frame_end.as_u8();
        buf.push(ZDLE);
        buf.push(end_byte);
        last = end_byte;
        crc = update_crc32(crc, end_byte);
        crc = !crc;

        for _ in 0..4 {
            let c = crc as u8;
            if c & 0x60 != 0 {
                // Non-control char: send raw (matches xsendline)
                buf.push(c);
                last = c;
            } else {
                Self::encode_into(&mut buf, c, &mut last, escape);
            }
            crc >>= 8;
        }

        if frame_end == FrameEnd::CrcW {
            buf.push(XON);
            last = XON;
        }

        out.write_all(&buf)?;
        self.last_sent = last;

        if frame_end == FrameEnd::CrcW {
            out.flush()?;
        }
        Ok(())
    }

    /// Send data — dispatches to 16-bit or 32-bit CRC.
    pub fn send_data<W: Write>(
        &mut self,
        data: &[u8],
        frame_end: FrameEnd,
        escape: &EscapeTable,
        out: &mut W,
    ) -> io::Result<()> {
        if self.use_crc32 {
            self.send_data32(data, frame_end, escape, out)
        } else {
            self.send_data16(data, frame_end, escape, out)
        }
    }

    /// Send cancel sequence (CAN * 10 + BS * 10).
    pub fn send_cancel<W: Write>(&mut self, out: &mut W) -> io::Result<()> {
        let cancel: [u8; 20] = [
            0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, // CAN * 10
            0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, // BS * 10
        ];
        out.flush()?;
        out.write_all(&cancel)?;
        out.flush()?;
        Ok(())
    }
}

/// Store a position into a 4-byte header (little-endian).
pub fn store_position(pos: u64) -> [u8; 4] {
    [
        pos as u8,
        (pos >> 8) as u8,
        (pos >> 16) as u8,
        (pos >> 24) as u8,
    ]
}

/// Recover position from a 4-byte header (little-endian).
pub fn recover_position(hdr: &[u8; 4]) -> u64 {
    hdr[0] as u64
        | (hdr[1] as u64) << 8
        | (hdr[2] as u64) << 16
        | (hdr[3] as u64) << 24
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::escape::EscapeTable;

    #[test]
    fn test_hex_header_format() {
        let mut encoder = FrameEncoder::new();
        let hdr = store_position(0);
        let mut buf = Vec::new();
        encoder
            .send_hex_header(FrameType::ZrqInit, &hdr, &mut buf)
            .unwrap();

        // Should start with ** ZDLE B (hex indicator)
        assert_eq!(buf[0], ZPAD);
        assert_eq!(buf[1], ZPAD);
        assert_eq!(buf[2], ZDLE);
        assert_eq!(buf[3], ZHEX);
        // Type 0 = "00"
        assert_eq!(buf[4], b'0');
        assert_eq!(buf[5], b'0');
    }

    #[test]
    fn test_bin16_header_format() {
        let escape = EscapeTable::new(false, false);
        let mut encoder = FrameEncoder::new();
        let hdr = [0u8; 4];
        let mut buf = Vec::new();
        encoder
            .send_bin16_header(FrameType::ZrInit, &hdr, 0, &escape, &mut buf)
            .unwrap();

        assert_eq!(buf[0], ZPAD);
        assert_eq!(buf[1], ZDLE);
        assert_eq!(buf[2], ZBIN);
    }

    #[test]
    fn test_position_roundtrip() {
        let pos: u64 = 0xDEAD_BEEF;
        let hdr = store_position(pos);
        let recovered = recover_position(&hdr);
        // Only 32 bits stored
        assert_eq!(recovered, pos & 0xFFFF_FFFF);
    }

    #[test]
    fn test_cancel_sequence() {
        let mut encoder = FrameEncoder::new();
        let mut buf = Vec::new();
        encoder.send_cancel(&mut buf).unwrap();
        assert_eq!(buf.len(), 20);
        assert!(buf[..10].iter().all(|&b| b == 0x18));
        assert!(buf[10..].iter().all(|&b| b == 0x08));
    }
}
