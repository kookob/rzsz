# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

rzsz is a Rust rewrite of lrzsz (X/Y/ZModem file transfer tool). It produces a single binary `zz` that handles both sending and receiving, with all other commands (`rz`, `sz`, `rrz`, `rsz`, etc.) as symlinks detected via argv[0].

## Build and Test

```bash
cargo build                    # dev build
cargo build --release          # optimized (opt-level=s, LTO, strip, panic=abort)
cargo test                     # unit tests
bash tests/interop.sh          # interop tests (requires lrzsz C binaries)

# static musl build
cargo build --release --target x86_64-unknown-linux-musl
```

Release binary target: < 500KB (musl static).

## Architecture

**Core principles: zero globals, explicit state machine, type-safe I/O separation.**

```
src/
├── bin/zz.rs           # Single binary: argv[0] → send/receive/protocol detection
├── sender.rs           # ZModem send: handshake, adaptive blocks, multi-file batch
├── receiver.rs         # ZModem receive: ZRINIT negotiation, ackbibi, resume
├── xmodem.rs           # XModem: 128B/1K blocks, CRC-16/checksum
├── ymodem.rs           # YModem: batch transfer, file headers, path sanitization
├── zmodem/
│   ├── frame.rs        # FrameEncoder, FrameType enum (20 types), protocol constants
│   ├── crc.rs          # CRC-16/CRC-32 lookup tables
│   ├── escape.rs       # ZDLE escape table (EscapeTable, turbo mode)
│   └── session.rs      # Session state machine, header receive, ZDLE decoding
└── serial/
    ├── mod.rs           # ProtocolWriter/StatusWriter newtype (compile-time I/O isolation)
    ├── reader.rs        # ModemReader: poll() timeout, unread_byte pushback
    └── terminal.rs      # TerminalGuard: RAII terminal mode restore (BorrowedFd)
```

## Key Design Notes

- argv[0] detection: `zz`=auto, `rz/rrz/lrz`=receive, `sz/rsz/lsz`=send, `rb/sb`=YModem, `rx/sx`=XModem
- CRC-16 formula: `table[(crc>>8)&255] ^ (crc<<8) ^ byte` (matches C macro exactly)
- ZModem header byte order: ZF0=hdr[3], ZF1=hdr[2], ZF2=hdr[1], ZF3=hdr[0]
- Session end: receiver must implement ackbibi() to consume sender's "OO" before terminal restore
- TerminalGuard uses BorrowedFd (not OwnedFd) to avoid closing stdin on drop
- CAN (0x18) and ZDLE (0x18) are same byte — receive_header resets CAN counter on ZPAD
- Default overwrite on receive; `-p` protects, `-E` renames

## Dependencies

- `nix` — termios, poll, signal (POSIX syscalls)
- `signal-hook` — safe signal handling
- `log` + `env_logger` — logging
