//! rrz — receive files with X/Y/ZModem protocol.
//! Part of the rzsz package, a Rust rewrite of lrzsz.

use std::env;
use std::io::{stdin, stdout};
use std::path::PathBuf;
use std::process;

use rzsz::serial::reader::ModemReader;
use rzsz::serial::terminal::TerminalGuard;
use rzsz::zmodem::session::Session;
use rzsz::receiver::{self, ReceiverConfig};

fn main() {
    let args: Vec<String> = env::args().collect();
    let program_name = args
        .first()
        .and_then(|a| a.rsplit('/').next())
        .unwrap_or("rrz");

    // Detect protocol from argv[0]
    let protocol = match program_name {
        "rrb" | "lrb" | "rb" => "ymodem",
        "rrx" | "lrx" | "rx" => "xmodem",
        _ => "zmodem",
    };

    if protocol != "zmodem" {
        eprintln!("{program_name}: {protocol} not yet implemented");
        process::exit(1);
    }

    // Set up terminal
    let _guard = TerminalGuard::new(0).ok();
    if let Some(ref guard) = _guard {
        let _ = guard.set_raw();
    }

    let stdin_fd = stdin();
    let mut reader = ModemReader::new(stdin_fd.lock(), 16384);
    let mut out = stdout().lock();
    let mut session = Session::new();
    let config = ReceiverConfig {
        output_dir: PathBuf::from("."),
        ..Default::default()
    };

    match receiver::receive_files(&mut session, &mut reader, &mut out, &config) {
        Ok(files) => {
            for f in &files {
                eprintln!("\rreceived: {f}");
            }
        }
        Err(rzsz::zmodem::session::ZError::Cancelled) => {
            // Normal end of session
        }
        Err(e) => {
            eprintln!("\r{program_name}: {e}");
            process::exit(1);
        }
    }
}
