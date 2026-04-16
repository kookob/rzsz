//! rsz — send files with X/Y/ZModem protocol.
//! Part of the rzsz package, a Rust rewrite of lrzsz.

use std::env;
use std::io::{stdin, stdout};
use std::path::Path;
use std::process;

use rzsz::serial::reader::ModemReader;
use rzsz::serial::terminal::TerminalGuard;
use rzsz::zmodem::session::Session;
use rzsz::sender::{self, SenderConfig};

fn main() {
    let args: Vec<String> = env::args().collect();
    let program_name = args
        .first()
        .and_then(|a| a.rsplit('/').next())
        .unwrap_or("rsz");

    // Detect protocol from argv[0]
    let protocol = match program_name {
        "rsb" | "lsb" | "sb" => "ymodem",
        "rsx" | "lsx" | "sx" => "xmodem",
        _ => "zmodem",
    };

    if protocol != "zmodem" {
        eprintln!("{program_name}: {protocol} not yet implemented");
        process::exit(1);
    }

    // Collect file arguments (skip program name and any options)
    let files: Vec<&str> = args.iter()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .map(|s| s.as_str())
        .collect();

    if files.is_empty() {
        eprintln!("usage: {program_name} [options] file...");
        process::exit(1);
    }

    // Set up terminal
    let _guard = unsafe { TerminalGuard::from_raw_fd(0) }.ok();
    if let Some(ref guard) = _guard {
        let _ = guard.set_raw();
    }

    // Set stderr unbuffered (already is by default in Rust)
    let stdin_fd = stdin();
    let mut reader = ModemReader::new(stdin_fd.lock(), 16384);
    let mut out = stdout().lock();
    let mut session = Session::new();
    let config = SenderConfig::default();

    // Get receiver init
    if let Err(e) = sender::get_receiver_init(&mut session, &mut reader, &mut out) {
        eprintln!("\r{program_name}: {e}");
        process::exit(1);
    }

    // Send each file
    let mut errors = 0;
    for file_path in &files {
        let path = Path::new(file_path);
        match sender::send_file(&mut session, &mut reader, &mut out, path, &config) {
            Ok(bytes) => {
                if bytes > 0 {
                    eprintln!("\r{}: {} bytes sent", file_path, bytes);
                }
            }
            Err(e) => {
                eprintln!("\r{program_name}: {file_path}: {e}");
                errors += 1;
            }
        }
    }

    // Finish session
    let _ = sender::finish_session(&mut session, &mut reader, &mut out);

    process::exit(if errors > 0 { 1 } else { 0 });
}
