//! ZModem receiver implementation.
//! Equivalent to the wcreceive/tryz/rzfile flow in lrz.c.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write, Seek, SeekFrom, BufWriter};
use std::os::unix::io::AsFd;
use std::path::{Path, PathBuf};

use crate::serial::reader::ModemReader;
use crate::zmodem::frame::*;
use crate::zmodem::session::*;

const MAX_BLOCK: usize = 8192;
const RETRY_MAX: u32 = 20;

/// Receiver configuration.
pub struct ReceiverConfig {
    /// Verbosity level (0 = normal, 1+ = increasingly verbose).
    pub verbosity: u8,
    /// Quiet mode — suppress progress output.
    pub quiet: bool,
    /// Protect existing files — never overwrite (lrz -p).
    pub protect: bool,
    /// Rename mode (-E): if file exists, generate unique name (.1, .2, etc.)
    pub rename: bool,
    pub resume: bool,
    pub restricted: bool,
    /// Force binary receive mode.
    pub binary: bool,
    /// Force ASCII (text) receive mode.
    pub ascii: bool,
    /// Escape all control characters.
    pub escape_ctrl: bool,
    pub junk_path: bool,
    pub output_dir: PathBuf,
}

impl Default for ReceiverConfig {
    fn default() -> Self {
        Self {
            verbosity: 0,
            quiet: false,
            protect: false,
            rename: false,
            resume: false,
            restricted: true,
            binary: true,
            ascii: false,
            escape_ctrl: false,
            junk_path: false,
            output_dir: PathBuf::from("."),
        }
    }
}

impl ReceiverConfig {
    /// Returns true if verbose output should be shown (verbosity >= 1 and not quiet).
    pub fn is_verbose(&self) -> bool {
        self.verbosity > 0 && !self.quiet
    }
}

/// Parsed file metadata from ZFILE header.
#[allow(dead_code)]
struct FileHeader {
    name: String,
    size: u64,
    mtime: u64,
    mode: u32,
}

/// Parse the ZFILE data subpacket.
fn parse_file_header(data: &[u8]) -> Option<FileHeader> {
    // Find NUL terminator after filename
    let nul_pos = data.iter().position(|&b| b == 0)?;
    let name = String::from_utf8_lossy(&data[..nul_pos]).to_string();

    // Parse metadata after NUL: "size mtime mode ..."
    let meta = &data[nul_pos + 1..];
    let meta_str = String::from_utf8_lossy(meta);
    let mut parts = meta_str.split_whitespace();

    let size = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let mtime = parts
        .next()
        .and_then(|s| u64::from_str_radix(s, 8).ok())
        .unwrap_or(0);
    let mode = parts
        .next()
        .and_then(|s| u32::from_str_radix(s, 8).ok())
        .unwrap_or(0o644);

    Some(FileHeader {
        name,
        size,
        mtime,
        mode,
    })
}

/// Check if path is safe in restricted mode (no .. components).
fn is_safe_path(path: &Path) -> bool {
    for component in path.components() {
        if let std::path::Component::ParentDir = component {
            return false;
        }
    }
    !path.is_absolute()
}

/// Acknowledge ZFIN and consume the sender's "OO" (Over-and-Out).
/// Equivalent to ackbibi() in lrz.c.
fn ackbibi<R: Read + AsFd, W: Write>(
    session: &Session,
    reader: &mut ModemReader<R>,
    out: &mut W,
) -> Result<(), ZError> {
    for _ in 0..3 {
        reader.purge();
        // Send our own ZFIN
        let hdr = store_position(0);
        // Use hex header like C lrz does
        let mut enc = FrameEncoder::new();
        enc.send_hex_header(FrameType::ZFin, &hdr, out)?;

        // Wait for 'O' (first byte of "OO")
        match reader.read_byte(session.rx_timeout_tenths) {
            Ok(b'O') => {
                // Consume second 'O'
                let _ = reader.read_byte(1);
                return Ok(());
            }
            Err(_) => continue, // timeout, retry
            _ => continue,
        }
    }
    Ok(())
}

/// Send ZRINIT and wait for sender's response.
/// Equivalent to tryz() in lrz.c.
/// Optimized: after receiving ZRQINIT, reads next header directly
/// without resending ZRINIT (ZFILE is likely already in the buffer).
pub fn try_zmodem<R: Read + AsFd, W: Write>(
    session: &mut Session,
    reader: &mut ModemReader<R>,
    out: &mut W,
) -> Result<ReceivedHeader, ZError> {
    let mut retries = 0u32;

    // Pre-build ZRINIT header (reused across retries)
    let mut flags = [0u8; 4];
    flags[3] = CANFDX | CANOVIO | CANBRK | CANFC32;
    if session.escape_all_ctrl {
        flags[3] |= ESCCTL;
    }
    let buflen = session.max_block_size as u16;
    flags[0] = (buflen >> 8) as u8;
    flags[1] = buflen as u8;

    // Send initial ZRINIT
    session.encoder.send_hex_header(FrameType::ZrInit, &flags, out)?;

    loop {
        // Read next header (C lrz: "again:" label)
        match session.receive_header(reader) {
            Ok(hdr) => match hdr.frame_type {
                FrameType::ZrqInit => {
                    // ZFILE typically follows in the same buffer.
                    // Resend ZRINIT and immediately read again.
                    session.encoder.send_hex_header(FrameType::ZrInit, &flags, out)?;
                    continue; // Go to receive_header, not rebuild flags
                }
                FrameType::ZFile => {
                    if hdr.encoding == FrameEncoding::Bin32 {
                        session.encoder.use_crc32 = true;
                    }
                    return Ok(hdr);
                }
                FrameType::ZsInit => {
                    let mut attn_buf = Vec::new();
                    let _ = session.receive_data16(reader, &mut attn_buf, ZATTNLEN);
                    session.attn = attn_buf;
                    continue;
                }
                FrameType::ZFin => {
                    ackbibi(session, reader, out)?;
                    return Err(ZError::Cancelled);
                }
                FrameType::ZFreeCnt => {
                    session.send_pos_header(FrameType::ZAck, 0x7FFF_FFFF, out)?;
                    continue;
                }
                FrameType::ZCommand => {
                    let hdr = [0u8; 4];
                    session.encoder.send_hex_header(FrameType::ZCompl, &hdr, out)?;
                    continue;
                }
                FrameType::ZEof => {
                    // Sender finished a file — respond with ZRINIT for next file
                    session.encoder.send_hex_header(FrameType::ZrInit, &flags, out)?;
                    continue;
                }
                FrameType::ZAbort => return Err(ZError::Cancelled),
                _ => {
                    retries += 1;
                }
            },
            Err(ZError::Timeout) => {
                retries += 1;
                // Resend ZRINIT on timeout
                session.encoder.send_hex_header(FrameType::ZrInit, &flags, out)?;
            }
            Err(e) => return Err(e),
        }

        if retries > RETRY_MAX {
            return Err(ZError::TooManyErrors);
        }
    }
}

/// Receive files via ZModem.
pub fn receive_files<R: Read + AsFd, W: Write>(
    session: &mut Session,
    reader: &mut ModemReader<R>,
    out: &mut W,
    config: &ReceiverConfig,
) -> Result<Vec<String>, ZError> {
    let mut received_files = Vec::new();

    loop {
        // Wait for ZFILE
        let _zfile_hdr = try_zmodem(session, reader, out)?;

        // Read file header data (CRC mode matches the ZFILE header encoding)
        let mut header_data = Vec::new();
        let _frame_end = if session.encoder.use_crc32 {
            session.receive_data32(reader, &mut header_data, MAX_BLOCK)?
        } else {
            session.receive_data16(reader, &mut header_data, MAX_BLOCK)?
        };

        let file_info = parse_file_header(&header_data)
            .ok_or_else(|| ZError::FrameError("invalid file header".into()))?;

        // Batch-end marker: sender sends an empty filename to signal no more files.
        if file_info.name.is_empty() {
            break Ok(received_files);
        }

        // Security: check path in restricted mode
        let file_path = if config.junk_path {
            config.output_dir.join(
                Path::new(&file_info.name)
                    .file_name()
                    .unwrap_or_default(),
            )
        } else {
            config.output_dir.join(&file_info.name)
        };

        if config.restricted && !is_safe_path(Path::new(&file_info.name)) {
            // Reject unsafe path — skip this file
            session.send_pos_header(FrameType::ZSkip, 0, out)?;
            continue;
        }

        // Determine start position (for resume)
        let start_pos = if config.resume {
            if let Ok(existing) = fs::metadata(&file_path) {
                existing.len()
            } else {
                0
            }
        } else {
            0
        };

        // Skip if path resolves to a directory (e.g. empty filename edge case)
        if file_path.is_dir() {
            session.send_pos_header(FrameType::ZSkip, 0, out)?;
            continue;
        }

        // Tell sender where to start
        session.send_pos_header(FrameType::ZRpos, start_pos, out)?;

        // Receive file data
        match receive_file_data(session, reader, out, &file_path, start_pos, file_info.size, config) {
            Ok(bytes) => {
                if bytes > 0 {
                    let msg = format!("{}: {} bytes received\n", file_info.name, bytes);
                    let _ = std::io::Write::write_all(&mut std::io::stderr(), msg.as_bytes());
                }
                received_files.push(file_info.name);
            }
            Err(ZError::Io(ref e)) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                break Ok(received_files);
            }
            Err(ZError::Cancelled) => {
                // User cancelled — send cancel and exit immediately
                let _ = session.encoder.send_cancel(&mut *out);
                if !file_info.name.is_empty() {
                    eprintln!("error receiving {}: transfer cancelled", file_info.name);
                }
                break Ok(received_files);
            }
            Err(ZError::Timeout) => {
                // Timeout likely means sender gave up — exit
                break Ok(received_files);
            }
            Err(e) => {
                if !file_info.name.is_empty() {
                    eprintln!("error receiving {}: {}", file_info.name, e);
                }
                drain_until_eof(session, reader);
            }
        }
    }
}

/// Receive data for a single file.
fn receive_file_data<R: Read + AsFd, W: Write>(
    session: &mut Session,
    reader: &mut ModemReader<R>,
    out: &mut W,
    path: &Path,
    start_pos: u64,
    _expected_size: u64,
    config: &ReceiverConfig,
) -> Result<u64, ZError> {
    // --protect: skip if file exists
    if path.exists() && config.protect {
        eprintln!("skipped (already exists): {}", path.display());
        session.send_pos_header(FrameType::ZSkip, 0, out)?;
        return Ok(0);
    }

    // --rename (-E): generate unique name if file exists
    let path = if path.exists() && config.rename {
        let mut candidate = path.to_path_buf();
        for i in 1..=9999u32 {
            candidate = PathBuf::from(format!("{}.{}", path.display(), i));
            if !candidate.exists() {
                break;
            }
        }
        eprintln!("rename: {} -> {}", path.display(), candidate.display());
        candidate
    } else {
        path.to_path_buf()
    };
    let path = path.as_path();

    // Default: overwrite existing file (matching C lrz behavior)

    // Create parent directories if needed
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            // Notify sender to skip this file before returning error
            let _ = session.send_pos_header(FrameType::ZSkip, 0, out);
            return Err(ZError::Io(e));
        }
    }

    let file = if start_pos > 0 {
        match OpenOptions::new().write(true).open(path) {
            Ok(f) => f,
            Err(e) => {
                let _ = session.send_pos_header(FrameType::ZSkip, 0, out);
                return Err(ZError::Io(e));
            }
        }
    } else {
        match File::create(path) {
            Ok(f) => f,
            Err(e) => {
                let _ = session.send_pos_header(FrameType::ZSkip, 0, out);
                return Err(ZError::Io(e));
            }
        }
    };

    let mut writer = BufWriter::new(file);
    if start_pos > 0 {
        if let Err(e) = writer.seek(SeekFrom::Start(start_pos)) {
            let _ = session.send_pos_header(FrameType::ZSkip, 0, out);
            return Err(ZError::Io(e));
        }
    }

    let mut position = start_pos;
    let mut retries = 0u32;
    let mut data_buf = Vec::with_capacity(MAX_BLOCK);

    loop {
        // Wait for ZDATA header
        let hdr = session.receive_header(reader)?;

        match hdr.frame_type {
            FrameType::ZData => {
                let data_pos = recover_position(&hdr.hdr);
                if data_pos != position {
                    // Position mismatch — request resync
                    session.send_pos_header(FrameType::ZRpos, position, out)?;
                    retries += 1;
                    if retries > RETRY_MAX {
                        return Err(ZError::TooManyErrors);
                    }
                    continue;
                }
            }
            FrameType::ZEof => {
                let eof_pos = recover_position(&hdr.hdr);
                if eof_pos == position {
                    writer.flush().map_err(ZError::Io)?;
                    return Ok(position - start_pos);
                }
                // Position mismatch on EOF — ignore and wait for more
                continue;
            }
            FrameType::ZFin => {
                writer.flush().map_err(ZError::Io)?;
                return Ok(position - start_pos);
            }
            _ => {
                continue;
            }
        }

        // Receive data blocks
        loop {
            let frame_end = if session.encoder.use_crc32 {
                session.receive_data32(reader, &mut data_buf, MAX_BLOCK)?
            } else {
                session.receive_data16(reader, &mut data_buf, MAX_BLOCK)?
            };

            if let Err(e) = writer.write_all(&data_buf) {
                // Local write failed — tell sender to skip this file
                let _ = session.send_pos_header(FrameType::ZSkip, 0, out);
                return Err(ZError::Io(e));
            }
            position += data_buf.len() as u64;
            retries = 0;

            match frame_end {
                FrameEnd::CrcW => {
                    // ACK required
                    session.send_pos_header(FrameType::ZAck, position, out)?;
                    break; // Back to header loop
                }
                FrameEnd::CrcQ => {
                    // ACK required but continue
                    session.send_pos_header(FrameType::ZAck, position, out)?;
                }
                FrameEnd::CrcG => {
                    // Continue without ACK (streaming)
                }
                FrameEnd::CrcE => {
                    // End of frame, header follows
                    break;
                }
            }
        }
    }
}

/// Drain frames from the sender until we see ZEOF or timeout.
/// Used after a local I/O error so the sender's current file stream
/// is consumed and we can proceed to the next file.
fn drain_until_eof<R: Read + AsFd>(
    session: &Session,
    reader: &mut ModemReader<R>,
) {
    let mut data_buf = Vec::with_capacity(MAX_BLOCK);
    // Try up to a generous number of iterations to find ZEOF
    for _ in 0..200 {
        match session.receive_header(reader) {
            Ok(hdr) if hdr.frame_type == FrameType::ZEof => return,
            Ok(hdr) if hdr.frame_type == FrameType::ZFin => return,
            Ok(hdr) if hdr.frame_type == FrameType::ZData => {
                // Drain the data subpackets within this ZDATA frame
                loop {
                    let frame_end = if session.encoder.use_crc32 {
                        session.receive_data32(reader, &mut data_buf, MAX_BLOCK)
                    } else {
                        session.receive_data16(reader, &mut data_buf, MAX_BLOCK)
                    };
                    match frame_end {
                        Ok(FrameEnd::CrcE) | Ok(FrameEnd::CrcW) => break,
                        Ok(_) => continue,
                        Err(_) => return,
                    }
                }
            }
            Ok(_) => continue,
            Err(_) => return,
        }
    }
}
