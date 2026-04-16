//! zz — unified send/receive files with X/Y/ZModem protocol.
//!
//! Combines rsz (send) and rrz (receive) into one binary:
//!   zz file1 file2   →  send (equivalent to sz)
//!   zz               →  receive (equivalent to rz)
//!   rz  (symlink)    →  force receive mode
//!   sz  (symlink)    →  force send mode
//!
//! Part of the rzsz package, a Rust rewrite of lrzsz.

use std::env;
use std::io::{self, stdin, stdout, Write};
use std::path::{Path, PathBuf};
use std::process;

use rzsz::serial::reader::ModemReader;
use rzsz::serial::terminal::TerminalGuard;
use rzsz::zmodem::session::Session;
use rzsz::sender::{self, SenderConfig};
use rzsz::receiver::{self, ReceiverConfig};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Detect mode from argv[0].
/// Returns Some(true) for forced send, Some(false) for forced receive, None for auto.
fn detect_mode(name: &str) -> Option<bool> {
    match name {
        "sz" | "rsz" | "lsz" | "sb" | "rsb" | "lsb" | "sx" | "rsx" | "lsx" => Some(true),
        "rz" | "rrz" | "lrz" | "rb" | "rrb" | "lrb" | "rx" | "rrx" | "lrx" => Some(false),
        _ => None, // "zz" or unknown → auto-detect
    }
}

fn print_usage(name: &str) {
    eprintln!("Usage: {name} [options] [file...]");
    eprintln!("  With files: send files (ZModem)");
    eprintln!("  Without files: receive files (ZModem)\n");
    eprintln!("Send options:");
    eprintln!("  -f, --full-path     send full pathname");
    eprintln!("  -T, --turbo         turbo escape mode");
    eprintln!("  -k, --1024          use 1024 byte blocks");
    eprintln!("  -8, --try-8k        try 8K blocks\n");
    eprintln!("Receive options:");
    eprintln!("  -y, --overwrite     overwrite existing files");
    eprintln!("  -p, --protect       protect existing files (don't overwrite)");
    eprintln!("  -j, --junk-path     junk pathname (save to current dir only)");
    eprintln!("  -R, --restricted    restricted mode (default, no .. in paths)");
    eprintln!("  -U, --unrestrict    disable restricted mode\n");
    eprintln!("Common options:");
    eprintln!("  -v, --verbose       increase verbosity (repeatable)");
    eprintln!("  -q, --quiet         quiet mode");
    eprintln!("  -b, --binary        binary mode");
    eprintln!("  -a, --ascii         ASCII mode");
    eprintln!("  -e, --escape        escape all control characters");
    eprintln!("  -r, --resume        resume interrupted transfer");
    eprintln!("  -h, --help          show this help");
    eprintln!("      --version       show version");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let program_name = args
        .first()
        .and_then(|a| a.rsplit('/').next())
        .unwrap_or("zz");

    let forced_mode = detect_mode(program_name);

    // Parse options (shared between send and receive)
    let mut send_cfg = SenderConfig::default();
    let mut recv_cfg = ReceiverConfig {
        output_dir: PathBuf::from("."),
        ..Default::default()
    };
    let mut files: Vec<String> = Vec::new();
    let mut i = 1;

    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            files.extend(args[i + 1..].iter().cloned());
            break;
        }
        match arg.as_str() {
            "-h" | "--help" => { print_usage(program_name); process::exit(0); }
            "--version" => { eprintln!("{program_name} {VERSION}"); process::exit(0); }
            // Common
            "-v" | "--verbose" => {
                send_cfg.verbosity = send_cfg.verbosity.saturating_add(1);
                recv_cfg.verbosity = recv_cfg.verbosity.saturating_add(1);
            }
            "-q" | "--quiet" => { send_cfg.quiet = true; recv_cfg.quiet = true; }
            "-b" | "--binary" => {
                send_cfg.binary = true; send_cfg.ascii = false;
                recv_cfg.binary = true; recv_cfg.ascii = false;
            }
            "-a" | "--ascii" => {
                send_cfg.ascii = true; send_cfg.binary = false;
                recv_cfg.ascii = true; recv_cfg.binary = false;
            }
            "-e" | "--escape" => { send_cfg.escape_ctrl = true; recv_cfg.escape_ctrl = true; }
            "-r" | "--resume" => { send_cfg.resume = true; recv_cfg.resume = true; }
            // Send-specific
            "-f" | "--full-path" => { send_cfg.full_path = true; }
            "-T" | "--turbo" => { send_cfg.turbo = true; }
            "-k" | "--1024" => { send_cfg.max_block = 1024; }
            "-8" | "--try-8k" => { send_cfg.max_block = 8192; }
            // Receive-specific
            "-y" | "--overwrite" => { recv_cfg.protect = false; recv_cfg.rename = false; }
            "-p" | "--protect" => { recv_cfg.protect = true; recv_cfg.rename = false; }
            "-j" | "--junk-path" => { recv_cfg.junk_path = true; }
            "-R" | "--restricted" => { recv_cfg.restricted = true; }
            "-U" | "--unrestrict" => { recv_cfg.restricted = false; }
            "-E" | "--rename" => { recv_cfg.rename = true; }
            // Combined short options
            other if other.starts_with('-') && !other.starts_with("--") && other.len() > 2 => {
                for ch in other[1..].chars() {
                    match ch {
                        'v' => { send_cfg.verbosity += 1; recv_cfg.verbosity += 1; }
                        'q' => { send_cfg.quiet = true; recv_cfg.quiet = true; }
                        'b' => { send_cfg.binary = true; recv_cfg.binary = true; }
                        'a' => { send_cfg.ascii = true; recv_cfg.ascii = true; }
                        'e' => { send_cfg.escape_ctrl = true; recv_cfg.escape_ctrl = true; }
                        'r' => { send_cfg.resume = true; recv_cfg.resume = true; }
                        'f' => send_cfg.full_path = true,
                        'T' => send_cfg.turbo = true,
                        'k' => send_cfg.max_block = 1024,
                        '8' => send_cfg.max_block = 8192,
                        'y' => { recv_cfg.protect = false; recv_cfg.rename = false; }
                        'p' => { recv_cfg.protect = true; recv_cfg.rename = false; }
                        'j' => recv_cfg.junk_path = true,
                        'R' => recv_cfg.restricted = true,
                        'U' => recv_cfg.restricted = false,
                        'E' => recv_cfg.rename = true,
                        'h' => { print_usage(program_name); process::exit(0); }
                        _ => {} // silently ignore for compat
                    }
                }
            }
            _ if arg.starts_with("--") => {} // ignore unknown long options
            _ if arg.starts_with('-') => {} // ignore unknown short options
            _ => { files.push(arg.clone()); }
        }
        i += 1;
    }

    // Determine mode: send or receive
    let is_send = match forced_mode {
        Some(send) => send,
        None => !files.is_empty(), // zz: auto-detect by presence of files
    };

    if is_send {
        if files.is_empty() {
            eprintln!("usage: {program_name} [options] file...");
            process::exit(1);
        }
        do_send(program_name, &files, &send_cfg);
    } else {
        do_receive(program_name, &recv_cfg);
    }
}

fn do_send(program_name: &str, files: &[String], config: &SenderConfig) -> ! {
    let guard = TerminalGuard::new(0).ok();
    if let Some(ref g) = guard { let _ = g.set_raw(); }

    let exit_code = {
        let stdin_fd = stdin();
        let mut reader = ModemReader::new(stdin_fd.lock(), 16384);
        let mut out = stdout().lock();
        let mut session = Session::new();

        if config.escape_ctrl {
            session.escape_all_ctrl = true;
        }
        if config.escape_ctrl || config.turbo {
            session.escape_table =
                rzsz::zmodem::escape::EscapeTable::new(config.escape_ctrl, config.turbo);
        }

        if let Err(e) = sender::get_receiver_init(&mut session, &mut reader, &mut out) {
            drop(out); drop(reader); drop(guard);
            eprintln!("{program_name}: {e}");
            process::exit(1);
        }

        let total_size: u64 = files.iter()
            .filter_map(|f| std::fs::metadata(f).ok())
            .map(|m| m.len()).sum();

        let mut errors = 0;
        let mut bytes_left = total_size;
        for (idx, file_path) in files.iter().enumerate() {
            let files_left = files.len() - idx;
            let path = Path::new(file_path);
            match sender::send_file(
                &mut session, &mut reader, &mut out, path, config,
                files_left, bytes_left, None,
            ) {
                Ok(bytes) => {
                    if bytes > 0 && !config.quiet {
                        eprintln!("\r{file_path}: {bytes} bytes sent");
                    }
                    bytes_left = bytes_left.saturating_sub(bytes);
                }
                Err(e) => {
                    eprintln!("\r{program_name}: {file_path}: {e}");
                    errors += 1;
                }
            }
        }

        let _ = sender::finish_session(&mut session, &mut reader, &mut out);
        if errors > 0 { 1 } else { 0 }
    };

    drop(guard);
    process::exit(exit_code);
}

fn do_receive(program_name: &str, config: &ReceiverConfig) {
    // Output trigger text for terminal emulators BEFORE raw mode
    {
        let mut out = stdout().lock();
        let _ = out.write_all(b"rz waiting to receive.\r\n");
        let _ = out.flush();
    }

    let mut guard = TerminalGuard::new(0).ok();
    if let Some(ref g) = guard { let _ = g.set_raw(); }

    let exit_code;
    {
        let stdin_fd = stdin();
        let mut reader = ModemReader::new(stdin_fd.lock(), 16384);
        let mut out = stdout().lock();
        let mut session = Session::new();

        if config.escape_ctrl {
            session.escape_all_ctrl = true;
            session.escape_table =
                rzsz::zmodem::escape::EscapeTable::new(true, false);
        }

        exit_code = match receiver::receive_files(&mut session, &mut reader, &mut out, config) {
            Ok(ref files) => {
                let names: Vec<String> = files.clone();
                // Drain trailing bytes (e.g. "OO" from sender) before restoring terminal
                for _ in 0..10 {
                    if reader.read_byte(2).is_err() { break; }
                }
                drop(out); drop(reader); guard.take();
                if !config.quiet {
                    for f in &names { eprintln!("received: {f}"); }
                }
                0
            }
            Err(rzsz::zmodem::session::ZError::Cancelled) => 0,
            Err(rzsz::zmodem::session::ZError::Io(ref e))
                if e.kind() == io::ErrorKind::BrokenPipe => 0,
            Err(ref e) => {
                let msg = format!("{program_name}: {e}");
                drop(out); drop(reader); guard.take();
                eprintln!("{msg}");
                1
            }
        };
    }

    drop(guard);
    if exit_code != 0 { process::exit(exit_code); }
}
