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

    if protocol != "zmodem" {
        eprintln!("{program_name}: {protocol} not yet implemented");
        process::exit(1);
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

    // Set up terminal
    let _guard = TerminalGuard::new(0).ok();
    if let Some(ref guard) = _guard {
        let _ = guard.set_raw();
    }

    // Set stderr unbuffered (already is by default in Rust)
    let stdin_fd = stdin();
    let mut reader = ModemReader::new(stdin_fd.lock(), 16384);
    let mut out = stdout().lock();
    let mut session = Session::new();

    // Apply escape option to session
    if config.escape_ctrl {
        session.escape_all_ctrl = true;
        session.escape_table = rzsz::zmodem::escape::EscapeTable::new(true, false);
    }

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
                if bytes > 0 && !config.quiet {
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
