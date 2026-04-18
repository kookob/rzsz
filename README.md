# rzsz

A modern Rust rewrite of the classic [lrzsz](https://ohse.de/uwe/software/lrzsz.html) file transfer tool. Transfer files over terminal connections using ZModem, XModem, and YModem protocols.

**[中文文档](README_CN.md)**

## Features

- **Single binary** — One `zz` binary (449KB static), all commands via symlinks
- **Drop-in replacement** — Works as `rz`, `sz`, `rrz`, `rsz` via argv[0] detection
- **Smart mode** — `zz file` sends, `zz` receives, no separate commands needed
- **ZModem** — CRC-16/32, adaptive block sizing, multi-file batch, crash recovery
- **XModem** — 128B/1K blocks, CRC-16 and checksum modes
- **YModem** — Batch transfer with file headers, size tracking
- **Secure** — Restricted mode by default, path traversal protection, filename sanitization
- **Terminal compatible** — Works with Xshell, SecureCRT, iTerm2, MobaXterm
- **Zero dependencies** — Static musl build, runs on any Linux (x86_64, aarch64)

## Quick Start

### One-line install (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/kookob/rzsz/main/install.sh | sudo bash
```

### Uninstall

```bash
curl -fsSL https://raw.githubusercontent.com/kookob/rzsz/main/install.sh | sudo bash -s -- --uninstall
```

### Other install methods

```bash
# From cargo
cargo install rzsz

# From binary release
curl -LO https://github.com/kookob/rzsz/releases/latest/download/zz-linux-x86_64-musl.tar.gz
sudo tar xzf zz-linux-x86_64-musl.tar.gz -C /usr/local/bin/

# Replace system rz/sz (optional)
sudo ln -sf /usr/local/bin/zz /usr/bin/rz
sudo ln -sf /usr/local/bin/zz /usr/bin/sz
```

## Usage

```bash
# Unified command (auto-detect mode)
zz file1 file2        # Send files
zz                    # Receive files

# Traditional commands (symlinks to zz)
sz file1 file2        # Send files
rz                    # Receive files

# Common options
zz -r file            # Resume interrupted transfer
zz -p                 # Receive: protect existing files (don't overwrite)
zz -E                 # Receive: rename if file exists (.1, .2, ...)
zz -e file            # Escape all control characters
zz -T file            # Turbo mode (less escaping, faster)
zz -8 file            # Try 8K blocks
zz -q file            # Quiet mode
zz -v file            # Verbose mode
zz --help             # Show all options
```

### File Overwrite Policy

| Option | Behavior |
|--------|----------|
| Default (no flags) | Overwrite existing files |
| `-p` / `--protect` | Skip existing files with message |
| `-E` / `--rename` | Auto-rename (file.1, file.2, ...) |

## How It Works

`zz` is a single binary that determines its behavior from how it is invoked:

| Invoked as | Mode | Protocol |
|------------|------|----------|
| `zz file` | Send | ZModem (auto) |
| `zz` | Receive | ZModem (auto) |
| `sz` / `rsz` / `lsz` | Send (forced) | ZModem |
| `rz` / `rrz` / `lrz` | Receive (forced) | ZModem |
| `sb` / `rsb` | Send | YModem |
| `rb` / `rrb` | Receive | YModem |
| `sx` / `rsx` | Send | XModem |
| `rx` / `rrx` | Receive | XModem |

### X/YModem limitations

X/YModem are provided for compatibility. Only ZModem mode honors the full option set. On X/YModem paths:

- **XModem send**: `-k` / `-8` selects 1024-byte blocks; other send options are ignored.
- **XModem receive**: the first non-flag argument is the destination filename (XModem has no filename in the protocol). Output is padded to the block boundary.
- **YModem receive**: `-q` suppresses the `received:` line; `-p` / `-y` / `-E` / `-r` / `-R` / `-U` have no effect (files are always overwritten into the current directory with the filename from block 0).

For full option support, use ZModem (the default).

## Comparison with lrzsz

| | rzsz | lrzsz |
|--|------|-------|
| Language | Rust | C |
| Binary size (static) | 449 KB | ~200 KB |
| Binary count | 1 (`zz`) | 2 (`lsz` + `lrz`) |
| Unified command | `zz` (auto-detect) | No |
| Memory safety | Compile-time guaranteed | Manual |
| Terminal restore | RAII (guaranteed on all exit paths) | Signal handler (fragile) |
| Timeout mechanism | `poll()` | `alarm()`/SIGALRM |
| State management | Struct fields | 50+ global variables |
| Protocol state machine | Explicit enum + match | Implicit goto chains |
| Path traversal protection | Default on | Opt-in |

### Performance (10MB pipe transfer)

| Sender | Receiver | Throughput |
|--------|----------|-----------|
| C lsz | C lrz | 116 MB/s |
| Rust sz | C lrz | 49 MB/s |
| C lsz | Rust rz | 45 MB/s |
| Rust sz | Rust rz | 29 MB/s |

> The Rust version is 2-3x slower in pipe benchmarks due to per-byte escape encoding (vs C's batch `zsendline_s`). In real-world SSH/serial transfers, network latency dominates and the difference is imperceptible.

## Terminal Compatibility

| Terminal | ZModem Support | Status |
|----------|---------------|--------|
| Xshell | Built-in | Tested |
| SecureCRT | Built-in | Compatible |
| iTerm2 | Configurable trigger | Compatible |
| MobaXterm | Built-in | Compatible |
| Tabby | Plugin | Compatible |
| Windows Terminal | None | Use `scp` instead |
| PuTTY | None | Use `scp` instead |

## Building from Source

```bash
git clone https://github.com/kookob/rzsz.git
cd rzsz

# Development build
cargo build

# Release build
cargo build --release

# Static musl build (no runtime dependencies)
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl

# Run tests
cargo test
bash tests/interop.sh
```

## Architecture

```
src/
├── bin/zz.rs           # Unified binary: auto-detect send/receive from argv[0]
├── sender.rs           # ZModem send: handshake, file data, adaptive blocks
├── receiver.rs         # ZModem receive: ZRINIT negotiation, file write, resume
├── xmodem.rs           # XModem: 128B/1K blocks, CRC-16/checksum
├── ymodem.rs           # YModem: batch transfer, file headers
├── zmodem/
│   ├── frame.rs        # Frame encoding/decoding (hex/bin16/bin32)
│   ├── session.rs      # Protocol state machine, header parsing
│   ├── crc.rs          # CRC-16 and CRC-32 lookup tables
│   └── escape.rs       # ZDLE escape table
└── serial/
    ├── mod.rs           # ProtocolWriter/StatusWriter (type-safe I/O separation)
    ├── reader.rs        # Buffered modem reader with poll() timeout
    └── terminal.rs      # TerminalGuard (RAII terminal mode restore)
```

## Publishing

See [PUBLISHING.md](PUBLISHING.md) for crates.io and GitHub Release instructions.

## License

Apache-2.0

## Credits

- Original rzsz by Chuck Forsberg (Omen Technology)
- lrzsz maintained by Uwe Ohse
- ZModem protocol specification (1988)
