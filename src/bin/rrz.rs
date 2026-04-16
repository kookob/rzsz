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

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_usage(program_name: &str) {
    eprintln!("Usage: {} [options]", program_name);
    eprintln!("Receive file(s) with X/Y/ZModem protocol.\n");
    eprintln!("Options:");
    eprintln!("  -v, --verbose       increase verbosity (repeatable)");
    eprintln!("  -q, --quiet         quiet mode, suppress progress output");
    eprintln!("  -b, --binary        binary receive mode");
    eprintln!("  -a, --ascii         ASCII receive mode");
    eprintln!("  -e, --escape        escape all control characters");
    eprintln!("  -r, --resume        resume interrupted transfer (crash recovery)");
    eprintln!("  -y, --overwrite     overwrite existing files");
    eprintln!("  -p, --protect       protect existing files (don't overwrite)");
    eprintln!("  -j, --junk-path     junk pathname (save to current dir only)");
    eprintln!("  -R, --restricted    restricted mode (no .. in paths)");
    eprintln!("  -h, --help          show this help");
    eprintln!("      --version       show version");
}

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

    if protocol == "xmodem" {
        // XModem receive: needs a destination filename argument
        let dest = args.get(1).map(|s| s.as_str()).unwrap_or("xmodem.out");
        let _guard = TerminalGuard::new(0).ok();
        if let Some(ref guard) = _guard { let _ = guard.set_raw(); }
        let stdin_fd = stdin();
        let mut reader = rzsz::serial::reader::ModemReader::new(stdin_fd.lock(), 16384);
        let mut out = stdout().lock();
        match rzsz::xmodem::xmodem_receive(&mut reader, &mut out, &PathBuf::from(dest), true) {
            Ok(bytes) => { eprintln!("\r{dest}: {bytes} bytes received"); }
            Err(e) => { eprintln!("\r{program_name}: {e}"); process::exit(1); }
        }
        process::exit(0);
    }
    if protocol == "ymodem" {
        let _guard = TerminalGuard::new(0).ok();
        if let Some(ref guard) = _guard { let _ = guard.set_raw(); }
        let stdin_fd = stdin();
        let mut reader = rzsz::serial::reader::ModemReader::new(stdin_fd.lock(), 16384);
        let mut out = stdout().lock();
        match rzsz::ymodem::ymodem_receive(&mut reader, &mut out, &PathBuf::from(".")) {
            Ok(files) => { for f in &files { eprintln!("\rreceived: {f}"); } }
            Err(e) => { eprintln!("\r{program_name}: {e}"); process::exit(1); }
        }
        process::exit(0);
    }

    // Parse command-line options
    let mut config = ReceiverConfig {
        output_dir: PathBuf::from("."),
        ..Default::default()
    };
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            break;
        }
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage(program_name);
                process::exit(0);
            }
            "--version" => {
                eprintln!("{} {}", program_name, VERSION);
                process::exit(0);
            }
            "-v" | "--verbose" => {
                config.verbosity = config.verbosity.saturating_add(1);
            }
            "-q" | "--quiet" => {
                config.quiet = true;
            }
            "-b" | "--binary" => {
                config.binary = true;
                config.ascii = false;
            }
            "-a" | "--ascii" => {
                config.ascii = true;
                config.binary = false;
            }
            "-e" | "--escape" => {
                config.escape_ctrl = true;
            }
            "-r" | "--resume" => {
                config.resume = true;
            }
            "-y" | "--overwrite" => {
                config.clobber = true;
                config.protect = false;
            }
            "-p" | "--protect" => {
                config.protect = true;
                config.clobber = false;
            }
            "-j" | "--junk-path" => {
                config.junk_path = true;
            }
            "-R" | "--restricted" => {
                config.restricted = true;
            }
            other if other.starts_with('-') && !other.starts_with("--") && other.len() > 2 => {
                // Handle combined short options like -vvbe
                let chars: Vec<char> = other[1..].chars().collect();
                for ch in chars {
                    match ch {
                        'v' => config.verbosity = config.verbosity.saturating_add(1),
                        'q' => config.quiet = true,
                        'b' => { config.binary = true; config.ascii = false; }
                        'a' => { config.ascii = true; config.binary = false; }
                        'e' => config.escape_ctrl = true,
                        'r' => config.resume = true,
                        'y' => { config.clobber = true; config.protect = false; }
                        'p' => { config.protect = true; config.clobber = false; }
                        'j' => config.junk_path = true,
                        'R' => config.restricted = true,
                        'h' => {
                            print_usage(program_name);
                            process::exit(0);
                        }
                        _ => {
                            eprintln!("{}: unknown option '-{}'", program_name, ch);
                            eprintln!("Try '{} --help' for more information.", program_name);
                            process::exit(1);
                        }
                    }
                }
            }
            other if other.starts_with("--") => {
                eprintln!("{}: unknown option '{}'", program_name, other);
                eprintln!("Try '{} --help' for more information.", program_name);
                process::exit(1);
            }
            _ => {
                // rrz doesn't take file arguments, ignore or warn
                eprintln!("{}: warning: ignoring argument '{}'", program_name, arg);
            }
        }
        i += 1;
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

    // Apply escape option to session
    if config.escape_ctrl {
        session.escape_all_ctrl = true;
        session.escape_table = rzsz::zmodem::escape::EscapeTable::new(true, false);
    }

    match receiver::receive_files(&mut session, &mut reader, &mut out, &config) {
        Ok(files) => {
            if !config.quiet {
                for f in &files {
                    eprintln!("\rreceived: {f}");
                }
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
