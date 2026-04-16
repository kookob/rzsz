//! rsz — send files with X/Y/ZModem protocol.
//! Part of the rzsz package, a Rust rewrite of lrzsz.

use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    let program_name = args
        .first()
        .and_then(|a| a.rsplit('/').next())
        .unwrap_or("rsz");

    // Detect protocol from argv[0]
    let _protocol = match program_name {
        "rsb" | "lsb" | "sb" => "ymodem",
        "rsx" | "lsx" | "sx" => "xmodem",
        _ => "zmodem",
    };

    eprintln!("rsz: ZModem sender (rzsz {}) — not yet implemented", env!("CARGO_PKG_VERSION"));
    process::exit(1);
}
