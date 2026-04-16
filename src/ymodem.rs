//! YModem protocol implementation.
//! YModem = XModem-1K with batch file transfer and file header (block 0).

use std::io::{self, Read, Write};
use std::fs::{self, File};
use std::os::unix::io::AsFd;
use std::path::Path;

use crate::serial::reader::ModemReader;
use crate::xmodem;
use crate::zmodem::crc::update_crc16;

const SOH: u8 = 0x01;
const STX: u8 = 0x02;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;
const CAN: u8 = 0x18;
const WANTCRC: u8 = b'C';

const TIMEOUT_TENTHS: u32 = 100;
const RETRY_MAX: u32 = 10;

/// Send files using YModem batch protocol.
pub fn ymodem_send<R: Read + AsFd, W: Write>(
    reader: &mut ModemReader<R>,
    out: &mut W,
    files: &[&Path],
) -> Result<u64, io::Error> {
    let mut total_bytes: u64 = 0;

    for path in files {
        let metadata = fs::metadata(path)?;
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".to_string());
        let file_size = metadata.len();

        // Wait for 'C' from receiver
        let use_crc = wait_for_crc(reader)?;

        // Send block 0: filename + size
        let header = build_ymodem_header(&file_name, file_size);
        send_ymodem_block(reader, out, &header, 0, use_crc)?;

        // Wait for ACK then C
        wait_for_byte(reader, ACK)?;
        let use_crc = wait_for_crc(reader)?;

        // Send file data using XModem-1K blocks (skip handshake)
        let bytes = xmodem::xmodem_send_blocks(reader, out, path, true, use_crc)?;
        total_bytes += bytes;
    }

    // Send empty block 0 to end batch
    let use_crc = wait_for_crc(reader)?;
    let empty_header = vec![0u8; 128];
    send_ymodem_block(reader, out, &empty_header, 0, use_crc)?;

    Ok(total_bytes)
}

/// Receive files using YModem batch protocol.
pub fn ymodem_receive<R: Read + AsFd, W: Write>(
    reader: &mut ModemReader<R>,
    out: &mut W,
    output_dir: &Path,
) -> Result<Vec<String>, io::Error> {
    let mut received_files = Vec::new();

    loop {
        // Send 'C' to request CRC mode
        out.write_all(&[WANTCRC])?;
        out.flush()?;

        // Receive block 0 (file header)
        let first = reader.read_byte(TIMEOUT_TENTHS)?;
        if first != SOH && first != STX {
            continue;
        }

        let block_size = if first == STX { 1024 } else { 128 };
        let (sectnum, header) = receive_raw_block(reader, block_size, true)?;

        // Validate that block 0 has sectnum == 0
        if sectnum != 0 {
            out.write_all(&[NAK])?;
            out.flush()?;
            continue;
        }

        // ACK block 0
        out.write_all(&[ACK])?;
        out.flush()?;

        // Parse filename from header
        let nul_pos = header.iter().position(|&b| b == 0).unwrap_or(0);
        if nul_pos == 0 {
            // Empty filename = end of batch
            break;
        }

        let raw_name = String::from_utf8_lossy(&header[..nul_pos]).to_string();

        // Sanitize filename: extract basename only, reject traversal attempts
        let file_name = match Path::new(&raw_name).file_name() {
            Some(base) => {
                let s = base.to_string_lossy().to_string();
                if s.is_empty() || s == ".." || s.starts_with('/') {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "invalid filename in YModem header",
                    ));
                }
                s
            }
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid filename in YModem header",
                ));
            }
        };

        // Parse file size (after NUL)
        let size_str_end = header[nul_pos + 1..]
            .iter()
            .position(|&b| b == b' ' || b == 0)
            .unwrap_or(0);
        let file_size: u64 = if size_str_end > 0 {
            String::from_utf8_lossy(&header[nul_pos + 1..nul_pos + 1 + size_str_end])
                .parse()
                .unwrap_or(0)
        } else {
            0
        };

        let dest = output_dir.join(&file_name);

        // Send 'C' to start data reception (skip handshake inside xmodem)
        out.write_all(&[WANTCRC])?;
        out.flush()?;

        // Receive file data (skip initial C/NAK handshake)
        let bytes = xmodem::xmodem_receive_blocks(reader, out, &dest, true)?;

        // Truncate to actual file size if known (remove XModem padding)
        if file_size > 0 && bytes > file_size {
            let f = File::options().write(true).open(&dest)?;
            f.set_len(file_size)?;
        }

        received_files.push(file_name);
    }

    Ok(received_files)
}

fn build_ymodem_header(filename: &str, filesize: u64) -> Vec<u8> {
    let mut buf = vec![0u8; 128];
    let name_bytes = filename.as_bytes();
    let name_len = name_bytes.len().min(buf.len() - 1);
    buf[..name_len].copy_from_slice(&name_bytes[..name_len]);
    buf[name_len] = 0;

    let size_str = format!("{}", filesize);
    let size_bytes = size_str.as_bytes();
    let meta_start = name_len + 1;
    let meta_len = size_bytes.len().min(buf.len() - meta_start);
    buf[meta_start..meta_start + meta_len].copy_from_slice(&size_bytes[..meta_len]);

    buf
}

fn wait_for_crc<R: Read + AsFd>(reader: &mut ModemReader<R>) -> Result<bool, io::Error> {
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
    Err(io::Error::new(io::ErrorKind::TimedOut, "no CRC request"))
}

fn wait_for_byte<R: Read + AsFd>(
    reader: &mut ModemReader<R>,
    expected: u8,
) -> Result<(), io::Error> {
    for _ in 0..RETRY_MAX {
        match reader.read_byte(TIMEOUT_TENTHS) {
            Ok(b) if b == expected => return Ok(()),
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
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("waiting for {:#x}", expected),
    ))
}

fn send_ymodem_block<R: Read + AsFd, W: Write>(
    reader: &mut ModemReader<R>,
    out: &mut W,
    data: &[u8],
    sectnum: u8,
    use_crc: bool,
) -> Result<(), io::Error> {
    let header_byte = if data.len() > 128 { STX } else { SOH };

    for _ in 0..RETRY_MAX {
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
            Ok(NAK) | Ok(WANTCRC) => continue,
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
    Err(io::Error::new(io::ErrorKind::Other, "too many retries"))
}

fn receive_raw_block<R: Read + AsFd>(
    reader: &mut ModemReader<R>,
    block_size: usize,
    use_crc: bool,
) -> Result<(u8, Vec<u8>), io::Error> {
    let sectnum = reader.read_byte(TIMEOUT_TENTHS)?;
    let complement = reader.read_byte(TIMEOUT_TENTHS)?;

    // Validate sector number and complement
    if sectnum.wrapping_add(complement) != 0xFF {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "block number complement mismatch",
        ));
    }

    let mut data = vec![0u8; block_size];
    for i in 0..block_size {
        data[i] = reader.read_byte(TIMEOUT_TENTHS)?;
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
    }

    Ok((sectnum, data))
}
