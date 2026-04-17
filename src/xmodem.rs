//! XModem protocol implementation.
//! Supports 128-byte and 1024-byte (XModem-1K) blocks with CRC-16 or checksum.

use std::io::{self, Read, Write, BufReader};
use std::fs::File;
use std::os::unix::io::AsFd;
use std::path::Path;

use crate::serial::reader::ModemReader;
use crate::zmodem::crc::update_crc16;

// Protocol bytes
const SOH: u8 = 0x01;   // Start of 128-byte block
const STX: u8 = 0x02;   // Start of 1024-byte block
const EOT: u8 = 0x04;   // End of transmission
const ACK: u8 = 0x06;   // Acknowledge
const NAK: u8 = 0x15;   // Negative acknowledge
const CAN: u8 = 0x18;   // Cancel
const WANTCRC: u8 = b'C'; // Request CRC mode

const RETRY_MAX: u32 = 10;
const TIMEOUT_TENTHS: u32 = 100; // 10 seconds

/// Result of receiving a single block.
enum BlockResult {
    /// New data block received.
    Data(Vec<u8>),
    /// Duplicate of the previous block (ACK but do not advance).
    Duplicate,
}

/// Send a file using XModem protocol.
pub fn xmodem_send<R: Read + AsFd, W: Write>(
    reader: &mut ModemReader<R>,
    out: &mut W,
    path: &Path,
    use_1k: bool,
) -> Result<u64, io::Error> {
    // Wait for NAK or 'C' from receiver
    let use_crc = wait_for_start(reader)?;

    xmodem_send_blocks(reader, out, path, use_1k, use_crc)
}

/// Send file data as XModem blocks without performing the initial handshake.
///
/// The caller is responsible for negotiating the start signal (C/NAK) and
/// passing the resulting `use_crc` flag.
pub fn xmodem_send_blocks<R: Read + AsFd, W: Write>(
    reader: &mut ModemReader<R>,
    out: &mut W,
    path: &Path,
    use_1k: bool,
    use_crc: bool,
) -> Result<u64, io::Error> {
    let file = File::open(path)?;
    let _file_size = file.metadata()?.len();
    let mut file = BufReader::new(file);

    let block_size: usize = if use_1k { 1024 } else { 128 };
    let mut buf = vec![0x1Au8; block_size]; // Pad with SUB (^Z)
    let mut sectnum: u8 = 1;
    let mut bytes_sent: u64 = 0;

    loop {
        // Fill buffer from file
        buf.fill(0x1A); // Pad with SUB
        let n = read_full(&mut file, &mut buf)?;
        if n == 0 {
            break;
        }

        // Send block with retries
        send_block(reader, out, &buf, sectnum, use_crc, block_size)?;

        bytes_sent += n as u64;
        sectnum = sectnum.wrapping_add(1);
    }

    // Send EOT
    for _ in 0..RETRY_MAX {
        out.write_all(&[EOT])?;
        out.flush()?;
        match reader.read_byte(TIMEOUT_TENTHS) {
            Ok(ACK) => return Ok(bytes_sent),
            Ok(NAK) => continue,
            _ => continue,
        }
    }

    Err(io::Error::new(io::ErrorKind::TimedOut, "EOT not acknowledged"))
}

/// Receive a file using XModem protocol.
pub fn xmodem_receive<R: Read + AsFd, W: Write>(
    reader: &mut ModemReader<R>,
    out: &mut W,
    dest: &Path,
    use_crc: bool,
) -> Result<u64, io::Error> {
    // Send NAK or 'C' to initiate transfer
    let start_byte = if use_crc { WANTCRC } else { NAK };
    out.write_all(&[start_byte])?;
    out.flush()?;

    xmodem_receive_blocks(reader, out, dest, use_crc)
}

/// Receive file data as XModem blocks without sending the initial C/NAK handshake.
///
/// The caller is responsible for sending the start signal before calling this
/// function.
pub fn xmodem_receive_blocks<R: Read + AsFd, W: Write>(
    reader: &mut ModemReader<R>,
    out: &mut W,
    dest: &Path,
    use_crc: bool,
) -> Result<u64, io::Error> {
    let mut file = File::create(dest)?;
    let mut sectnum: u8 = 1;
    let mut bytes_received: u64 = 0;

    loop {
        let first = match reader.read_byte(TIMEOUT_TENTHS) {
            Ok(b) => b,
            Err(_) => {
                out.write_all(&[NAK])?;
                out.flush()?;
                continue;
            }
        };

        match first {
            SOH | STX => {
                let block_size = if first == STX { 1024 } else { 128 };
                match receive_block(reader, block_size, sectnum, use_crc) {
                    Ok(BlockResult::Data(data)) => {
                        file.write_all(&data)?;
                        bytes_received += data.len() as u64;
                        sectnum = sectnum.wrapping_add(1);
                        out.write_all(&[ACK])?;
                        out.flush()?;
                    }
                    Ok(BlockResult::Duplicate) => {
                        // ACK but do not advance sectnum or byte counter
                        out.write_all(&[ACK])?;
                        out.flush()?;
                    }
                    Err(_) => {
                        // Flush remaining bytes and NAK
                        reader.purge();
                        out.write_all(&[NAK])?;
                        out.flush()?;
                    }
                }
            }
            EOT => {
                out.write_all(&[ACK])?;
                out.flush()?;
                break;
            }
            CAN => {
                // Require two consecutive CAN bytes to cancel
                if let Ok(CAN) = reader.read_byte(TIMEOUT_TENTHS) {
                    return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "cancelled"));
                }
            }
            _ => continue,
        }
    }

    Ok(bytes_received)
}

fn wait_for_start<R: Read + AsFd>(reader: &mut ModemReader<R>) -> Result<bool, io::Error> {
    for _ in 0..RETRY_MAX {
        match reader.read_byte(TIMEOUT_TENTHS) {
            Ok(WANTCRC) => return Ok(true),
            Ok(NAK) => return Ok(false),
            Ok(CAN) => {
                // Require two consecutive CAN bytes to cancel
                if let Ok(CAN) = reader.read_byte(TIMEOUT_TENTHS) {
                    return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "cancelled"));
                }
                // Single CAN — ignore and keep waiting
                continue;
            }
            _ => continue,
        }
    }
    Err(io::Error::new(io::ErrorKind::TimedOut, "no start signal"))
}

fn send_block<R: Read + AsFd, W: Write>(
    reader: &mut ModemReader<R>,
    out: &mut W,
    data: &[u8],
    sectnum: u8,
    use_crc: bool,
    block_size: usize,
) -> Result<(), io::Error> {
    let header_byte = if block_size == 1024 { STX } else { SOH };

    for attempt in 0..RETRY_MAX {
        out.write_all(&[header_byte, sectnum, !sectnum])?;
        out.write_all(data)?;

        if use_crc {
            let mut crc: u16 = 0;
            for &b in data {
                crc = update_crc16(crc, b);
            }
            crc = update_crc16(update_crc16(crc, 0), 0);
            out.write_all(&[(crc >> 8) as u8, crc as u8])?;
        } else {
            let checksum: u8 = data.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
            out.write_all(&[checksum])?;
        }
        out.flush()?;

        match reader.read_byte(TIMEOUT_TENTHS) {
            Ok(ACK) => return Ok(()),
            Ok(NAK) | Ok(WANTCRC) => {
                if attempt == 0 && use_crc {
                    // First NAK after CRC might mean receiver wants checksum
                }
                continue;
            }
            Ok(CAN) => {
                // Require two consecutive CAN bytes to cancel
                if let Ok(CAN) = reader.read_byte(TIMEOUT_TENTHS) {
                    return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "cancelled"));
                }
                // Single CAN — treat as noise, retry
                continue;
            }
            _ => continue,
        }
    }
    Err(io::Error::other("too many retries"))
}

fn receive_block<R: Read + AsFd>(
    reader: &mut ModemReader<R>,
    block_size: usize,
    expected_sectnum: u8,
    use_crc: bool,
) -> Result<BlockResult, io::Error> {
    let sectnum = reader.read_byte(TIMEOUT_TENTHS)?;
    let complement = reader.read_byte(TIMEOUT_TENTHS)?;

    if sectnum.wrapping_add(complement) != 0xFF {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad sector number"));
    }

    let mut data = vec![0u8; block_size];
    for item in data.iter_mut() {
        *item = reader.read_byte(TIMEOUT_TENTHS)?;
    }

    if use_crc {
        let crc_hi = reader.read_byte(TIMEOUT_TENTHS)?;
        let crc_lo = reader.read_byte(TIMEOUT_TENTHS)?;
        let mut crc: u16 = 0;
        for &b in &data {
            crc = update_crc16(crc, b);
        }
        crc = update_crc16(crc, crc_hi);
        crc = update_crc16(crc, crc_lo);
        if crc != 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "CRC error"));
        }
    } else {
        let recv_checksum = reader.read_byte(TIMEOUT_TENTHS)?;
        let calc_checksum: u8 = data.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
        if recv_checksum != calc_checksum {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "checksum error"));
        }
    }

    if sectnum != expected_sectnum {
        if sectnum == expected_sectnum.wrapping_sub(1) {
            // Duplicate block — ACK and ignore
            return Ok(BlockResult::Duplicate);
        }
        return Err(io::Error::new(io::ErrorKind::InvalidData, "out of sequence"));
    }

    Ok(BlockResult::Data(data))
}

fn read_full<R: Read>(reader: &mut R, buf: &mut [u8]) -> io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match reader.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}
