//! ZModem receiver implementation.
//! Equivalent to the wcreceive/tryz/rzfile flow in lrz.c.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write, Seek, SeekFrom, BufWriter};
use std::os::unix::ffi::OsStrExt;
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
    /// Raw filename bytes, kept verbatim like C lrz. Terminal emulators on
    /// Windows often send GBK, not UTF-8; decoding lossily would destroy it.
    name: Vec<u8>,
    size: u64,
    mtime: u64,
    mode: u32,
}

impl FileHeader {
    fn display(&self) -> String {
        String::from_utf8_lossy(&self.name).into_owned()
    }
}

/// Parse the ZFILE data subpacket.
fn parse_file_header(data: &[u8]) -> Option<FileHeader> {
    // Find NUL terminator after filename
    let nul_pos = data.iter().position(|&b| b == 0)?;
    let name = data[..nul_pos].to_vec();

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

/// Build ZRINIT flags for this session. Matches C lrz format:
/// ZF0 = CANFDX | CANOVIO | CANFC32 (+ ESCCTL if needed),
/// buflen = 0 (streaming mode — sender chooses block size).
fn build_zrinit_flags(session: &Session) -> [u8; 4] {
    let mut flags = [0u8; 4];
    flags[3] = CANFDX | CANOVIO | CANFC32;
    if session.escape_all_ctrl {
        flags[3] |= ESCCTL;
    }
    // buflen = 0 → let sender stream. (C lrz does this.)
    flags
}

/// Send a burst of ZRINIT frames to announce readiness.
/// C lrz emits 5 ZRINITs back-to-back at startup so that when the
/// terminal emulator's sz starts after the user picks a file, it
/// sees one immediately instead of waiting for its own retry timeout.
pub fn send_zrinit_burst<W: Write>(
    session: &mut Session,
    out: &mut W,
    count: u32,
) -> Result<(), ZError> {
    let flags = build_zrinit_flags(session);
    for _ in 0..count {
        session.encoder.send_hex_header(FrameType::ZrInit, &flags, out)?;
    }
    Ok(())
}

/// Errors that end the session immediately (everything else is retried).
fn is_fatal(e: &ZError) -> bool {
    matches!(e, ZError::Io(_) | ZError::Cancelled)
}

/// Send ZRINIT and wait for the sender's ZFILE. Returns the ZFILE data
/// subpacket (filename + metadata), or an empty Vec once the sender's ZFIN
/// has been answered (session over).
/// Equivalent to tryz() in lrz.c.
pub fn try_zmodem<R: Read + AsFd, W: Write>(
    session: &mut Session,
    reader: &mut ModemReader<R>,
    out: &mut W,
) -> Result<Vec<u8>, ZError> {
    let mut retries = 0u32;

    let flags = build_zrinit_flags(session);

    // Send initial ZRINIT (caller may have already burst several — that's fine)
    session.encoder.send_hex_header(FrameType::ZrInit, &flags, out)?;

    let mut data = Vec::with_capacity(MAX_BLOCK);
    loop {
        // Read next header (C lrz: "again:" label)
        match session.receive_header(reader) {
            Ok(hdr) => {
                let crc32 = hdr.encoding == FrameEncoding::Bin32;
                match hdr.frame_type {
                    FrameType::ZrqInit => {
                        // ZFILE typically follows in the same buffer.
                        // Resend ZRINIT and immediately read again.
                        session.encoder.send_hex_header(FrameType::ZrInit, &flags, out)?;
                        continue;
                    }
                    FrameType::ZFile => {
                        match session.receive_data(reader, &mut data, MAX_BLOCK, crc32) {
                            Ok(FrameEnd::CrcW) => return Ok(data),
                            Err(e) if is_fatal(&e) => return Err(e),
                            // Bad subpacket: ZNAK, sender resends ZFILE (C lrz).
                            _ => session.send_pos_header(FrameType::ZNak, 0, out)?,
                        }
                        retries += 1;
                    }
                    FrameType::ZsInit => {
                        // Sender's attn string; must be ZACKed or the sender
                        // keeps retrying ZSINIT (C lrz: zshhdr(ZACK, 1)).
                        let mut attn = Vec::new();
                        match session.receive_data(reader, &mut attn, ZATTNLEN, crc32) {
                            Ok(FrameEnd::CrcW) => {
                                session.attn = attn;
                                session.send_pos_header(FrameType::ZAck, 1, out)?;
                            }
                            Err(e) if is_fatal(&e) => return Err(e),
                            _ => session.send_pos_header(FrameType::ZNak, 0, out)?,
                        }
                        continue;
                    }
                    FrameType::ZFin => {
                        ackbibi(session, reader, out)?;
                        return Ok(Vec::new());
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
                    FrameType::ZAbort | FrameType::ZCan => return Err(ZError::Cancelled),
                    _ => {
                        retries += 1;
                    }
                }
            }
            Err(e) if is_fatal(&e) => return Err(e),
            Err(_) => {
                // Timeout, bad CRC or garbage: resend ZRINIT (C lrz loops to
                // the top of tryz, which re-emits it).
                retries += 1;
                session.encoder.send_hex_header(FrameType::ZrInit, &flags, out)?;
            }
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

    // Startup burst: 5 back-to-back ZRINITs so the terminal emulator's
    // sz (spawned AFTER the user picks a file) sees one immediately
    // instead of waiting for its own retry timeout (typically 1-2s).
    // This matches C lrz behavior and is the single biggest factor in
    // "time from file pick to transfer start".
    send_zrinit_burst(session, out, 5)?;

    loop {
        // Wait for ZFILE (empty data = ZFIN handshake done, session over)
        let header_data = try_zmodem(session, reader, out)?;
        if header_data.is_empty() {
            break Ok(received_files);
        }

        let file_info = match parse_file_header(&header_data) {
            Some(info) => info,
            None => {
                // C lrz: procheader error -> ZSKIP
                session.send_pos_header(FrameType::ZSkip, 0, out)?;
                continue;
            }
        };

        // Batch-end marker: sender sends an empty filename to signal no more files.
        if file_info.name.is_empty() {
            break Ok(received_files);
        }

        let name = Path::new(OsStr::from_bytes(&file_info.name));

        // Security: check path in restricted mode
        let file_path = if config.junk_path {
            config.output_dir.join(name.file_name().unwrap_or_default())
        } else {
            config.output_dir.join(name)
        };

        if config.restricted && !is_safe_path(name) {
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
                    let msg = format!("{}: {} bytes received\n", file_info.display(), bytes);
                    let _ = std::io::Write::write_all(&mut std::io::stderr(), msg.as_bytes());
                }
                received_files.push(file_info.display());
            }
            Err(ZError::Io(ref e)) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                break Ok(received_files);
            }
            Err(ZError::Cancelled) => {
                // User cancelled — send cancel and exit immediately
                let _ = session.encoder.send_cancel(&mut *out);
                eprintln!("error receiving {}: transfer cancelled", file_info.display());
                break Ok(received_files);
            }
            Err(ZError::Io(e)) => {
                // Local write failed (ZSKIP already sent); the sender may
                // still be streaming this file — consume it, then go on.
                eprintln!("error receiving {}: {}", file_info.display(), e);
                drain_until_eof(session, reader);
            }
            // Protocol failure after all retries: caller cancels the session.
            Err(e) => return Err(e),
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
        // Wait for ZDATA header. Timeout, bad CRC or garbage is not fatal:
        // C lrz (rzfile) answers with ZRPOS so the sender resyncs.
        let hdr = match session.receive_header(reader) {
            Ok(hdr) => hdr,
            Err(e) if is_fatal(&e) => return Err(e),
            Err(_) => {
                retries += 1;
                if retries > RETRY_MAX {
                    return Err(ZError::TooManyErrors);
                }
                session.send_pos_header(FrameType::ZRpos, position, out)?;
                continue;
            }
        };
        let crc32 = hdr.encoding == FrameEncoding::Bin32;

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
            let frame_end = match session.receive_data(reader, &mut data_buf, MAX_BLOCK, crc32) {
                Ok(end) => end,
                Err(e) if is_fatal(&e) => return Err(e),
                Err(_) => {
                    // CRC error / timeout mid-frame: resync from what we have
                    retries += 1;
                    if retries > RETRY_MAX {
                        return Err(ZError::TooManyErrors);
                    }
                    session.send_pos_header(FrameType::ZRpos, position, out)?;
                    break; // back to header loop
                }
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
                let crc32 = hdr.encoding == FrameEncoding::Bin32;
                loop {
                    let frame_end = session.receive_data(reader, &mut data_buf, MAX_BLOCK, crc32);
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
