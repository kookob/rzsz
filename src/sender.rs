//! ZModem sender implementation.
//! Equivalent to the wcsend/wcs/wctxpn/zsendfdata flow in lsz.c.

use std::fs::{self, File};
use std::io::{Read, Write, Seek, SeekFrom, BufReader};
use std::os::unix::io::AsFd;
use std::path::Path;
use std::time::Instant;

use crate::serial::reader::ModemReader;
use crate::zmodem::frame::*;
use crate::zmodem::session::*;

const MAX_BLOCK: usize = 8192;
const RETRY_MAX: u32 = 10;

/// Sender configuration.
pub struct SenderConfig {
    pub verbose: bool,
    pub full_path: bool,
    pub resume: bool,
    pub escape_ctrl: bool,
    pub turbo: bool,
    pub max_block: usize,
    pub window_size: u32,
}

impl Default for SenderConfig {
    fn default() -> Self {
        Self {
            verbose: false,
            full_path: false,
            resume: false,
            escape_ctrl: false,
            turbo: false,
            max_block: 1024,
            window_size: 0,
        }
    }
}

/// Adaptive block size calculator.
/// Equivalent to calc_blklen() in lsz.c.
struct BlockSizer {
    blklen: usize,
    max_blklen: usize,
    total_errors: u64,
    total_sent: u64,
    last_error_pos: u64,
}

impl BlockSizer {
    fn new(max_blklen: usize) -> Self {
        let initial = if max_blklen >= 1024 { 1024 } else { 128 };
        Self {
            blklen: initial,
            max_blklen,
            total_errors: 0,
            total_sent: 0,
            last_error_pos: 0,
        }
    }

    fn current(&self) -> usize {
        self.blklen
    }

    fn record_error(&mut self) {
        self.total_errors += 1;
        // Halve block size on error, minimum 128
        self.blklen = (self.blklen / 2).max(128);
        self.last_error_pos = self.total_sent;
    }

    fn record_success(&mut self, bytes: usize) {
        self.total_sent += bytes as u64;
        // Grow block size if no recent errors
        if self.total_sent - self.last_error_pos > (self.blklen as u64 * 16) {
            if self.blklen < self.max_blklen {
                self.blklen = (self.blklen * 2).min(self.max_blklen);
            }
        }
    }
}

/// Wait for receiver init (ZRINIT). Equivalent to getzrxinit() in lsz.c.
pub fn get_receiver_init<R: Read + AsFd, W: Write>(
    session: &mut Session,
    reader: &mut ModemReader<R>,
    out: &mut W,
) -> Result<(), ZError> {
    let mut retries = 0u32;

    loop {
        // Send ZRQINIT
        session.send_pos_header(FrameType::ZrqInit, 0, out)?;

        match session.receive_header(reader) {
            Ok(hdr) => match hdr.frame_type {
                FrameType::ZrInit => {
                    // Parse receiver capabilities from header
                    let rx_flags = hdr.hdr[0]; // ZF0: capabilities
                    let rx_buflen = ((hdr.hdr[1] as u16) << 8) | hdr.hdr[2] as u16;

                    if rx_flags & CANFC32 != 0 {
                        session.encoder.use_crc32 = true;
                    }
                    if rx_flags & ESCCTL != 0 {
                        session.escape_all_ctrl = true;
                        session.escape_table =
                            crate::zmodem::escape::EscapeTable::new(true, false);
                    }
                    if rx_buflen > 0 {
                        session.max_block_size =
                            (rx_buflen as usize).min(MAX_BLOCK);
                    }

                    return Ok(());
                }
                FrameType::ZChallenge => {
                    // Echo back the challenge value
                    session.send_pos_header(
                        FrameType::ZAck,
                        recover_position(&hdr.hdr),
                        out,
                    )?;
                    continue;
                }
                FrameType::ZAbort | FrameType::ZFin => {
                    return Err(ZError::Cancelled);
                }
                _ => {
                    retries += 1;
                }
            },
            Err(ZError::Timeout) => {
                retries += 1;
            }
            Err(e) => return Err(e),
        }

        if retries > RETRY_MAX {
            return Err(ZError::TooManyErrors);
        }
    }
}

/// Build the ZFILE data subpacket: filename + metadata.
fn build_file_header(path: &Path, metadata: &fs::Metadata, full_path: bool) -> Vec<u8> {
    let mut buf = vec![0u8; MAX_BLOCK];

    // Filename (without path unless full_path)
    let name = if full_path {
        path.to_string_lossy().to_string()
    } else {
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unnamed".to_string())
    };

    let name_bytes = name.as_bytes();
    let name_len = name_bytes.len().min(buf.len() - 1);
    buf[..name_len].copy_from_slice(&name_bytes[..name_len]);
    buf[name_len] = 0; // NUL terminator

    // Metadata after filename: "size mtime mode 0 files_left total_left"
    let size = metadata.len();
    let mtime = {
        use std::time::UNIX_EPOCH;
        metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0)
    };
    let mode = {
        use std::os::unix::fs::MetadataExt;
        metadata.mode()
    };

    let meta_str = format!("{} {:o} {:o} 0 1 {}", size, mtime, mode, size);
    let meta_start = name_len + 1;
    let meta_bytes = meta_str.as_bytes();
    let meta_len = meta_bytes.len().min(buf.len() - meta_start);
    buf[meta_start..meta_start + meta_len].copy_from_slice(&meta_bytes[..meta_len]);

    // Truncate to used length (pad with zeros to at least 128 bytes for compat)
    let total_len = (meta_start + meta_len + 1).max(128);
    buf.truncate(total_len);
    buf
}

/// Send a single file via ZModem.
pub fn send_file<R: Read + AsFd, W: Write>(
    session: &mut Session,
    reader: &mut ModemReader<R>,
    out: &mut W,
    path: &Path,
    config: &SenderConfig,
) -> Result<u64, ZError> {
    let metadata = fs::metadata(path).map_err(|e| ZError::Io(e))?;
    if metadata.is_dir() {
        return Err(ZError::FrameError(format!("{}: is a directory", path.display())));
    }

    let file_size = metadata.len();
    let file_header = build_file_header(path, &metadata, config.full_path);

    // Send ZFILE header
    let hdr = [0u8; 4]; // ZF0-ZF3 (basic file options)
    session.encoder.send_binary_header(
        FrameType::ZFile,
        &hdr,
        0,
        &session.escape_table.clone(),
        out,
    )?;

    // Send file header data
    let escape = session.escape_table.clone();
    session.encoder.send_data(
        &file_header,
        FrameEnd::CrcW,
        &escape,
        out,
    )?;

    // Wait for receiver response
    loop {
        match session.receive_header(reader)? {
            hdr if hdr.frame_type == FrameType::ZrInit => {
                // Receiver didn't get our ZFILE, resend
                continue;
            }
            hdr if hdr.frame_type == FrameType::ZSkip => {
                return Ok(0); // Receiver skipped this file
            }
            hdr if hdr.frame_type == FrameType::ZRpos => {
                // Receiver wants data from this position
                let start_pos = recover_position(&hdr.hdr);
                return send_file_data(session, reader, out, path, start_pos, file_size, config);
            }
            hdr => {
                return Err(ZError::FrameError(format!(
                    "unexpected frame: {}",
                    hdr.frame_type.name()
                )));
            }
        }
    }
}

/// Send file data starting from a position.
fn send_file_data<R: Read + AsFd, W: Write>(
    session: &mut Session,
    reader: &mut ModemReader<R>,
    out: &mut W,
    path: &Path,
    start_pos: u64,
    file_size: u64,
    config: &SenderConfig,
) -> Result<u64, ZError> {
    let mut file = BufReader::new(File::open(path).map_err(ZError::Io)?);
    if start_pos > 0 {
        file.seek(SeekFrom::Start(start_pos)).map_err(ZError::Io)?;
    }

    let mut sizer = BlockSizer::new(config.max_block.min(MAX_BLOCK));
    let mut buf = vec![0u8; MAX_BLOCK];
    let mut position = start_pos;
    let mut bytes_sent: u64 = 0;
    let _start_time = Instant::now();

    // Send ZDATA header with starting position
    let escape = session.escape_table.clone();
    session.encoder.send_binary_header(
        FrameType::ZData,
        &store_position(position),
        0,
        &escape,
        out,
    )?;

    loop {
        let blklen = sizer.current();
        let to_read = blklen.min((file_size - position) as usize);
        if to_read == 0 {
            break;
        }

        let n = file.get_mut().read(&mut buf[..to_read]).map_err(ZError::Io)?;
        if n == 0 {
            break; // EOF
        }

        // Determine frame end type
        let at_eof = position + n as u64 >= file_size;
        let frame_end = if at_eof {
            FrameEnd::CrcE
        } else {
            FrameEnd::CrcG // Continue without ACK for streaming
        };

        let escape = session.escape_table.clone();
        session.encoder.send_data(&buf[..n], frame_end, &escape, out)?;
        position += n as u64;
        bytes_sent += n as u64;
        sizer.record_success(n);

        if at_eof {
            break;
        }
    }

    // Send ZEOF
    session.send_pos_header(FrameType::ZEof, position, out)?;

    // Wait for response
    let mut retries = 0u32;
    loop {
        match session.receive_header(reader) {
            Ok(hdr) => match hdr.frame_type {
                FrameType::ZrInit => {
                    // Receiver ready for next file — success
                    return Ok(bytes_sent);
                }
                FrameType::ZAck => {
                    return Ok(bytes_sent);
                }
                FrameType::ZRpos => {
                    // Receiver wants resync — in a full implementation
                    // we'd seek back and retransmit from that position
                    retries += 1;
                    if retries > RETRY_MAX {
                        return Err(ZError::TooManyErrors);
                    }
                    sizer.record_error();

                    let new_pos = recover_position(&hdr.hdr);
                    return send_file_data(session, reader, out, path, new_pos, file_size, config);
                }
                FrameType::ZSkip => return Ok(0),
                _ => {
                    retries += 1;
                    if retries > RETRY_MAX {
                        return Err(ZError::TooManyErrors);
                    }
                }
            },
            Err(ZError::Timeout) => {
                retries += 1;
                if retries > RETRY_MAX {
                    return Err(ZError::TooManyErrors);
                }
                // Resend ZEOF
                session.send_pos_header(FrameType::ZEof, position, out)?;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Send ZFIN to end the session.
pub fn finish_session<R: Read + AsFd, W: Write>(
    session: &mut Session,
    reader: &mut ModemReader<R>,
    out: &mut W,
) -> Result<(), ZError> {
    session.send_pos_header(FrameType::ZFin, 0, out)?;

    for _ in 0..5 {
        match session.receive_header(reader) {
            Ok(hdr) if hdr.frame_type == FrameType::ZFin => {
                // Send Over-and-Out
                out.write_all(b"OO")?;
                out.flush()?;
                return Ok(());
            }
            _ => continue,
        }
    }
    Ok(())
}
