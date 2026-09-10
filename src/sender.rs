//! ZModem sender implementation.
//! Equivalent to the wcsend/wcs/wctxpn/zsendfdata flow in lsz.c.

use std::fs::{self, File};
use std::io::{Read, Write, Seek, SeekFrom, BufReader};
use std::os::unix::io::AsFd;
use std::path::Path;

use crate::serial::reader::ModemReader;
use crate::zmodem::frame::*;
use crate::zmodem::session::*;

const MAX_BLOCK: usize = 8192;
const RETRY_MAX: u32 = 10;

/// Sender configuration.
pub struct SenderConfig {
    /// Verbosity level (0 = normal, 1+ = increasingly verbose).
    pub verbosity: u8,
    /// Quiet mode — suppress progress output.
    pub quiet: bool,
    /// Force binary transfer mode.
    pub binary: bool,
    /// Force ASCII (text) transfer mode.
    pub ascii: bool,
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
            verbosity: 0,
            quiet: false,
            binary: true,
            ascii: false,
            full_path: false,
            resume: false,
            escape_ctrl: false,
            turbo: false,
            max_block: 1024,
            window_size: 0,
        }
    }
}

impl SenderConfig {
    /// Returns true if verbose output should be shown (verbosity >= 1 and not quiet).
    pub fn is_verbose(&self) -> bool {
        self.verbosity > 0 && !self.quiet
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
        if self.total_sent - self.last_error_pos > (self.blklen as u64 * 16)
            && self.blklen < self.max_blklen
        {
            self.blklen = (self.blklen * 2).min(self.max_blklen);
        }
    }
}

/// Wait for receiver init (ZRINIT). Equivalent to getzrxinit() in lsz.c.
/// Maximum ZRQINIT attempts before giving up. Matches C lsz so terminal
/// emulators that pop a "Save as..." dialog (Xshell, SecureCRT) have time
/// to respond with ZRINIT.
const INIT_RETRIES: u32 = 14;
/// Timeout per init attempt in tenths of seconds (10 seconds).
const INIT_TIMEOUT_TENTHS: u32 = 100;

pub fn get_receiver_init<R: Read + AsFd, W: Write>(
    session: &mut Session,
    reader: &mut ModemReader<R>,
    out: &mut W,
) -> Result<(), ZError> {
    let mut retries = 0u32;
    

    // Send "rz\r" to trigger auto-start on the receiving end
    out.write_all(b"rz\r")?;
    out.flush()?;

    loop {
        // Send ZRQINIT
        session.send_pos_header(FrameType::ZrqInit, 0, out)?;

        // Use short timeout during init for responsive cancel
        let saved_timeout = session.rx_timeout_tenths;
        session.rx_timeout_tenths = INIT_TIMEOUT_TENTHS;
        let result = session.receive_header(reader);
        session.rx_timeout_tenths = saved_timeout;

        match result {
            Ok(hdr) => match hdr.frame_type {
                FrameType::ZrInit => {
                    let rx_flags = hdr.hdr[3];
                    // ZP0 = low byte, ZP1 = high byte (C: Rxhdr[ZP0] + Rxhdr[ZP1]<<8)
                    let rx_buflen = hdr.hdr[0] as usize | (hdr.hdr[1] as usize) << 8;

                    if rx_flags & CANFC32 != 0 {
                        session.encoder.use_crc32 = true;
                    }
                    if rx_flags & ESCCTL != 0 {
                        session.escape_all_ctrl = true;
                        session.escape_table =
                            crate::zmodem::escape::EscapeTable::new(true, false);
                    }
                    // Receiver buffer limit caps our block size (C lsz:
                    // blklen = min(blklen, Rxbuflen)); 0 means streaming.
                    session.max_block_size = if rx_buflen >= 32 {
                        rx_buflen.min(MAX_BLOCK)
                    } else {
                        MAX_BLOCK
                    };

                    return Ok(());
                }
                FrameType::ZChallenge => {
                    session.send_pos_header(
                        FrameType::ZAck,
                        recover_position(&hdr.hdr),
                        out,
                    )?;
                    continue;
                }
                FrameType::ZAbort | FrameType::ZFin | FrameType::ZCan => {
                    return Err(ZError::Cancelled);
                }
                _ => {
                    retries += 1;
                }
            },
            Err(ZError::Cancelled) => {
                // CAN*5 detected by frame parser
                return Err(ZError::Cancelled);
            }
            Err(ZError::Timeout) => {
                retries += 1;
            }
            Err(ZError::GarbageCount(_)) => {
                // Lots of garbage — might be terminal noise after cancel.
                // Check if we've seen CAN bytes in the garbage by trying
                // to read a few more bytes quickly.
                retries += 1;
            }
            Err(e) => return Err(e),
        }

        if retries > INIT_RETRIES {
            return Err(ZError::TooManyErrors);
        }
    }
}

/// Build the ZFILE data subpacket: filename + metadata.
/// `remote_name` overrides the filename sent to receiver (for directory transfers).
fn build_file_header(
    path: &Path,
    metadata: &fs::Metadata,
    full_path: bool,
    remote_name: Option<&str>,
    files_left: usize,
    total_bytes_left: u64,
) -> Vec<u8> {
    let mut buf = vec![0u8; MAX_BLOCK];

    // Filename: use remote_name if provided, else full path or basename
    let name = if let Some(rname) = remote_name {
        rname.to_string()
    } else if full_path {
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

    let meta_str = format!(
        "{} {:o} {:o} 0 {} {}",
        size, mtime, mode, files_left, total_bytes_left
    );
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
/// `remote_name` optionally overrides the filename sent to receiver.
#[allow(clippy::too_many_arguments)]
pub fn send_file<R: Read + AsFd, W: Write>(
    session: &mut Session,
    reader: &mut ModemReader<R>,
    out: &mut W,
    path: &Path,
    config: &SenderConfig,
    files_left: usize,
    total_bytes_left: u64,
    remote_name: Option<&str>,
) -> Result<u64, ZError> {
    let metadata = fs::metadata(path).map_err(ZError::Io)?;
    if metadata.is_dir() {
        return Err(ZError::FrameError(format!("{}: is a directory", path.display())));
    }

    let file_size = metadata.len();
    let file_header = build_file_header(
        path, &metadata, config.full_path, remote_name, files_left, total_bytes_left,
    );

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

    // Wait for receiver response.
    // Tolerate extra ZRINIT frames: C lrz sends 5 ZRINITs back-to-back
    // at startup, and any leftovers get queued in our input buffer.
    // Seeing ZRINIT here is NOT a "resend ZFILE" signal — only a timeout
    // means the receiver actually lost our ZFILE. The protocol-level
    // response to ZFILE is always ZRPOS/ZSKIP/ZCRC.
    let mut resend_retries = 0u32;
    loop {
        match session.receive_header(reader) {
            Ok(hdr) => match hdr.frame_type {
                FrameType::ZrInit => continue, // leftover burst — ignore
                FrameType::ZSkip => return Ok(0),
                FrameType::ZRpos => {
                    let start_pos = recover_position(&hdr.hdr);
                    return send_file_data(session, reader, out, path, start_pos, file_size, config);
                }
                FrameType::ZCrc => continue, // resume-mode CRC request, ignore
                _ => continue, // unknown / stray frame — keep waiting
            },
            Err(ZError::Timeout) => {
                resend_retries += 1;
                if resend_retries > RETRY_MAX {
                    return Err(ZError::TooManyErrors);
                }
                // Receiver actually lost our ZFILE — resend
                let hdr_bytes = [0u8; 4];
                let escape = session.escape_table.clone();
                session.encoder.send_binary_header(
                    FrameType::ZFile, &hdr_bytes, 0, &escape, out,
                )?;
                session.encoder.send_data(
                    &file_header, FrameEnd::CrcW, &escape, out,
                )?;
            }
            Err(e) => return Err(e),
        }
    }
}

/// What the receiver said on the reverse channel while we were streaming.
enum RxEvent {
    Nothing,
    /// ZRPOS: restart from this offset (the latest one queued wins).
    Resync(u64),
    /// ZSKIP / ZRINIT: receiver is done with this file.
    Skip,
    /// ZACK for the position we were waiting on (wait_ack only).
    Ack,
}

/// Block until the receiver acknowledges `position` (after a ZCRCW block).
/// Equivalent to getinsync(zi, 0) at lsz.c's waitack label. While we wait
/// we are reading, not pumping data, so stale ZRPOS frames get consumed
/// here instead of each one triggering another burst.
fn wait_ack<R: Read + AsFd, W: Write>(
    session: &mut Session,
    reader: &mut ModemReader<R>,
    out: &mut W,
    position: u64,
    file_size: u64,
) -> Result<RxEvent, ZError> {
    loop {
        match session.receive_header(reader) {
            Ok(hdr) => match hdr.frame_type {
                FrameType::ZAck => {
                    if recover_position(&hdr.hdr) == position {
                        return Ok(RxEvent::Ack);
                    }
                }
                FrameType::ZRpos => {
                    let pos = recover_position(&hdr.hdr);
                    if pos > file_size {
                        return Err(ZError::FrameError("ZRPOS beyond file size".into()));
                    }
                    return Ok(RxEvent::Resync(pos));
                }
                FrameType::ZSkip | FrameType::ZrInit => return Ok(RxEvent::Skip),
                FrameType::ZFin | FrameType::ZCan | FrameType::ZAbort => {
                    return Err(ZError::Cancelled)
                }
                _ => {}
            },
            Err(e @ (ZError::Io(_) | ZError::Cancelled | ZError::Timeout)) => return Err(e),
            Err(_) => session.send_pos_header(FrameType::ZNak, 0, out)?,
        }
    }
}

/// Drain pending input without blocking and interpret any headers in it.
/// Equivalent to the rdchk()/getinsync() loops in lsz.c zsendfdata: the
/// receiver may have queued several ZRPOS while it was skipping in-flight
/// data; we must consume them all and act on the last one, or each stale
/// ZRPOS drags the restart position backwards one at a time.
fn poll_receiver<R: Read + AsFd, W: Write>(
    session: &mut Session,
    reader: &mut ModemReader<R>,
    out: &mut W,
    file_size: u64,
) -> Result<RxEvent, ZError> {
    let mut event = RxEvent::Nothing;
    while reader.data_available() {
        let c = reader.read_byte(1)?;
        match c {
            ZPAD | 0x18 => {
                reader.unread_byte(c);
                match session.receive_header(reader) {
                    Ok(hdr) => match hdr.frame_type {
                        FrameType::ZRpos => {
                            let pos = recover_position(&hdr.hdr);
                            if pos > file_size {
                                return Err(ZError::FrameError("ZRPOS beyond file size".into()));
                            }
                            event = RxEvent::Resync(pos);
                        }
                        FrameType::ZAck => {}
                        FrameType::ZSkip | FrameType::ZrInit => return Ok(RxEvent::Skip),
                        FrameType::ZFin | FrameType::ZCan | FrameType::ZAbort => {
                            return Err(ZError::Cancelled)
                        }
                        _ => {}
                    },
                    Err(e @ (ZError::Io(_) | ZError::Cancelled | ZError::Timeout)) => return Err(e),
                    // Garbled header: C getinsync answers ZNAK and keeps reading
                    Err(_) => session.send_pos_header(FrameType::ZNak, 0, out)?,
                }
            }
            // Fake XOFF from the line: wait a while for the XON
            XOFF | 0x93 => {
                let _ = reader.read_byte(100);
            }
            _ => {} // junk
        }
    }
    Ok(event)
}

/// Send file data starting from a position. Uses iterative resync (no recursion).
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
    let mut sizer = BlockSizer::new(config.max_block.min(session.max_block_size));
    let mut buf = vec![0u8; MAX_BLOCK];
    let mut position = start_pos;
    let mut bytes_sent: u64 = 0;
    let mut retries = 0u32;
    // Offset of the last ZRPOS (C lsz: Lastsync). The first block sent from
    // there goes out as ZCRCW and we wait for its ZACK: one round trip that
    // re-establishes sync instead of streaming blindly into a receiver that
    // may still be discarding stale data and repeating its ZRPOS.
    let mut last_sync: Option<u64> = None;

    // Outer loop: handles ZRPOS resync by seeking and restarting data send
    'resync: loop {
        // Anything the receiver queued meanwhile? Take the last ZRPOS.
        match poll_receiver(session, reader, out, file_size)? {
            RxEvent::Resync(pos) => {
                position = pos;
                last_sync = Some(pos);
            }
            RxEvent::Skip => return Ok(0),
            RxEvent::Nothing | RxEvent::Ack => {}
        }

        file.seek(SeekFrom::Start(position)).map_err(ZError::Io)?;

        // Send ZDATA header with current position
        let escape = session.escape_table.clone();
        session.encoder.send_binary_header(
            FrameType::ZData,
            &store_position(position),
            0,
            &escape,
            out,
        )?;

        // Inner loop: send data blocks
        loop {
            let blklen = sizer.current();
            let remaining = file_size.saturating_sub(position) as usize;
            let to_read = blklen.min(remaining);
            if to_read == 0 {
                break;
            }

            let n = file.get_mut().read(&mut buf[..to_read]).map_err(ZError::Io)?;
            if n == 0 {
                break;
            }

            let at_eof = position + n as u64 >= file_size;
            let frame_end = if at_eof {
                FrameEnd::CrcE
            } else if last_sync == Some(position) {
                FrameEnd::CrcW // C: bytcnt == Lastsync -> ZCRCW, then waitack
            } else {
                FrameEnd::CrcG
            };

            let escape = session.escape_table.clone();
            session.encoder.send_data(&buf[..n], frame_end, &escape, out)?;
            position += n as u64;
            bytes_sent += n as u64;
            sizer.record_success(n);

            if at_eof {
                break;
            }

            if frame_end == FrameEnd::CrcW {
                match wait_ack(session, reader, out, position, file_size)? {
                    RxEvent::Ack => continue 'resync, // new ZDATA frame from here
                    RxEvent::Resync(pos) => {
                        sizer.record_error();
                        position = pos;
                        last_sync = Some(pos);
                        continue 'resync;
                    }
                    RxEvent::Skip => return Ok(0),
                    RxEvent::Nothing => unreachable!(),
                }
            }

            // Streaming: peek at the reverse channel between blocks so a
            // ZRPOS (receiver lost data) is honoured now, not after the
            // whole file has gone out. C lsz: rdchk() loop in zsendfdata.
            out.flush()?;
            match poll_receiver(session, reader, out, file_size)? {
                RxEvent::Resync(pos) => {
                    // Close the open frame, then restart (C: ZSDATA(txbuf,
                    // 0, ZCRCE) before resync). Like C lsz, ZRPOS is never
                    // counted toward giving up: the receiver may repeat it
                    // while stale data is still in flight.
                    let escape = session.escape_table.clone();
                    session.encoder.send_data(&[], FrameEnd::CrcE, &escape, out)?;
                    sizer.record_error();
                    position = pos;
                    last_sync = Some(pos);
                    continue 'resync;
                }
                RxEvent::Skip => return Ok(0),
                RxEvent::Nothing | RxEvent::Ack => {}
            }
        }

        // Send ZEOF (binary header, like C lsz)
        session.send_bin_pos_header(FrameType::ZEof, position, out)?;

        // Wait for response
        loop {
            match session.receive_header(reader) {
                Ok(hdr) => match hdr.frame_type {
                    FrameType::ZrInit => {
                        return Ok(bytes_sent);
                    }
                    FrameType::ZRpos => {
                        let new_pos = recover_position(&hdr.hdr);
                        // Validate position
                        if new_pos > file_size {
                            return Err(ZError::FrameError(
                                "ZRPOS beyond file size".into(),
                            ));
                        }
                        sizer.record_error();
                        position = new_pos;
                        last_sync = Some(new_pos);
                        continue 'resync; // Iterative resync
                    }
                    FrameType::ZSkip => return Ok(0),
                    FrameType::ZFin | FrameType::ZCan | FrameType::ZAbort => {
                        return Err(ZError::Cancelled)
                    }
                    // ZACK (of a ZCRCQ/ZCRCW) is not the answer to ZEOF:
                    // C lsz resends ZEOF and keeps waiting for ZRINIT.
                    _ => {
                        retries += 1;
                        if retries > RETRY_MAX {
                            return Err(ZError::TooManyErrors);
                        }
                        session.send_bin_pos_header(FrameType::ZEof, position, out)?;
                    }
                },
                Err(ZError::Timeout) => {
                    retries += 1;
                    if retries > RETRY_MAX {
                        return Err(ZError::TooManyErrors);
                    }
                    // Resend ZEOF
                    session.send_bin_pos_header(FrameType::ZEof, position, out)?;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// End the ZModem session with proper ZFIN handshake.
pub fn finish_session<R: Read + AsFd, W: Write>(
    session: &mut Session,
    reader: &mut ModemReader<R>,
    out: &mut W,
) -> Result<(), ZError> {
    let saved_timeout = session.rx_timeout_tenths;
    session.rx_timeout_tenths = 30; // 3 seconds for end handshake

    // Send ZFIN, retry a few times
    for _ in 0..4 {
        session.send_pos_header(FrameType::ZFin, 0, out)?;

        match session.receive_header(reader) {
            Ok(hdr) if hdr.frame_type == FrameType::ZFin => {
                // Receiver acknowledged — send Over-and-Out
                out.write_all(b"OO")?;
                out.flush()?;
                session.rx_timeout_tenths = saved_timeout;
                return Ok(());
            }
            Ok(hdr) if hdr.frame_type == FrameType::ZrInit => {
                // Receiver wants another file — send ZFIN again
                continue;
            }
            _ => continue,
        }
    }

    // Last resort: just send OO and exit
    out.write_all(b"OO")?;
    out.flush()?;
    session.rx_timeout_tenths = saved_timeout;
    Ok(())
}
