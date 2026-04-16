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

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_usage(program_name: &str) {
    eprintln!("Usage: {} [options] file...", program_name);
    eprintln!("Send file(s) with X/Y/ZModem protocol.\n");
    eprintln!("Options:");
    eprintln!("  -v, --verbose       increase verbosity (repeatable)");
    eprintln!("  -q, --quiet         quiet mode, suppress progress output");
    eprintln!("  -b, --binary        binary transfer mode");
    eprintln!("  -a, --ascii         ASCII transfer mode");
    eprintln!("  -e, --escape        escape all control characters");
    eprintln!("  -r, --resume        resume interrupted transfer");
    eprintln!("  -f, --full-path     send full pathname");
    eprintln!("  -T, --turbo         turbo escape mode");
    eprintln!("  -k, --1024          use 1024 byte blocks (default)");
    eprintln!("  -8, --try-8k        try 8K blocks");
    eprintln!("  -h, --help          show this help");
    eprintln!("      --version       show version");
}

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

    // XModem and YModem paths
    if protocol == "xmodem" {
        // Parse args, get first file
        let files: Vec<String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).cloned().collect();
        if files.is_empty() {
            eprintln!("usage: {program_name} file");
            process::exit(1);
        }
        let _guard = TerminalGuard::new(0).ok();
        if let Some(ref guard) = _guard { let _ = guard.set_raw(); }
        let stdin_fd = stdin();
        let mut reader = rzsz::serial::reader::ModemReader::new(stdin_fd.lock(), 16384);
        let mut out = stdout().lock();
        match rzsz::xmodem::xmodem_send(&mut reader, &mut out, Path::new(&files[0]), false) {
            Ok(bytes) => { eprintln!("\r{}: {} bytes sent", files[0], bytes); }
            Err(e) => { eprintln!("\r{program_name}: {e}"); process::exit(1); }
        }
        process::exit(0);
    }
    if protocol == "ymodem" {
        let file_args: Vec<String> = args.iter().skip(1).filter(|a| !a.starts_with('-')).cloned().collect();
        if file_args.is_empty() {
            eprintln!("usage: {program_name} file...");
            process::exit(1);
        }
        let _guard = TerminalGuard::new(0).ok();
        if let Some(ref guard) = _guard { let _ = guard.set_raw(); }
        let stdin_fd = stdin();
        let mut reader = rzsz::serial::reader::ModemReader::new(stdin_fd.lock(), 16384);
        let mut out = stdout().lock();
        let paths: Vec<&Path> = file_args.iter().map(|s| Path::new(s.as_str())).collect();
        match rzsz::ymodem::ymodem_send(&mut reader, &mut out, &paths) {
            Ok(bytes) => { eprintln!("\r{} bytes sent", bytes); }
            Err(e) => { eprintln!("\r{program_name}: {e}"); process::exit(1); }
        }
        process::exit(0);
    }

    // Parse command-line options
    let mut config = SenderConfig::default();
    let mut files: Vec<String> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            // Everything after "--" is a filename
            files.extend(args[i + 1..].iter().cloned());
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
            "-f" | "--full-path" => {
                config.full_path = true;
            }
            "-T" | "--turbo" => {
                config.turbo = true;
            }
            "-k" | "--1024" => {
                config.max_block = 1024;
            }
            "-8" | "--try-8k" => {
                config.max_block = 8192;
            }
            other if other.starts_with('-') && !other.starts_with("--") && other.len() > 2 => {
                // Handle combined short options like -vvb
                let chars: Vec<char> = other[1..].chars().collect();
                for ch in chars {
                    match ch {
                        'v' => config.verbosity = config.verbosity.saturating_add(1),
                        'q' => config.quiet = true,
                        'b' => { config.binary = true; config.ascii = false; }
                        'a' => { config.ascii = true; config.binary = false; }
                        'e' => config.escape_ctrl = true,
                        'r' => config.resume = true,
                        'f' => config.full_path = true,
                        'T' => config.turbo = true,
                        'k' => config.max_block = 1024,
                        '8' => config.max_block = 8192,
                        'h' => {
                            print_usage(program_name);
                            process::exit(0);
                        }
                        _ => {} // silently ignore unknown short options for compat
                    }
                }
            }
            _ if arg.starts_with("--") => {} // silently ignore unknown long options
            _ => {
                // Not an option — it's a filename
                files.push(arg.clone());
            }
        }
        i += 1;
    }

    if files.is_empty() {
        eprintln!("usage: {} [options] file...", program_name);
        process::exit(1);
    }

    // Set up terminal — guard must be dropped before exit to restore terminal
    let guard = TerminalGuard::new(0).ok();
    if let Some(ref g) = guard {
        let _ = g.set_raw();
    }

    let exit_code = {
        let stdin_fd = stdin();
        let mut reader = ModemReader::new(stdin_fd.lock(), 16384);
        let mut out = stdout().lock();
        let mut session = Session::new();

        if config.escape_ctrl {
            session.escape_all_ctrl = true;
            session.escape_table = rzsz::zmodem::escape::EscapeTable::new(true, false);
        }

        // Get receiver init
        if let Err(e) = sender::get_receiver_init(&mut session, &mut reader, &mut out) {
            drop(out);
            drop(reader);
            drop(guard); // Restore terminal BEFORE printing error
            eprintln!("{program_name}: {e}");
            process::exit(1);
        }

        // Compute batch totals
        let total_size: u64 = files
            .iter()
            .filter_map(|f| std::fs::metadata(f).ok())
            .map(|m| m.len())
            .sum();

        // Send each file
        let mut errors = 0;
        let mut bytes_left = total_size;
        for (idx, file_path) in files.iter().enumerate() {
            let files_left = files.len() - idx;
            let path = Path::new(file_path);
            match sender::send_file(
                &mut session, &mut reader, &mut out, path, &config,
                files_left, bytes_left, None,
            ) {
                Ok(bytes) => {
                    if bytes > 0 && !config.quiet {
                        eprintln!("\r{}: {} bytes sent", file_path, bytes);
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

    // Guard drops here — terminal restored before exit
    drop(guard);
    process::exit(exit_code);
}
