# rzsz

A modern Rust rewrite of the classic [lrzsz](https://ohse.de/uwe/software/lrzsz.html) file transfer tool. Transfer files over terminal connections using ZModem, XModem, and YModem protocols.

**[中文文档](#中文)**

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

### Install from cargo

```bash
cargo install rzsz
```

### Install from binary

Download from [Releases](https://github.com/nicholasching/rzsz/releases), then:

```bash
tar xzf zz-linux-x86_64-musl.tar.gz -C /usr/local/bin/
```

### Create system symlinks (optional, replaces system rz/sz)

```bash
sudo ln -sf /usr/local/bin/zz /usr/bin/rz
sudo ln -sf /usr/local/bin/zz /usr/bin/sz
```

## Usage

```bash
# Unified command
zz file1 file2        # Send files
zz                    # Receive files

# Traditional commands (symlinks to zz)
sz file1 file2        # Send files
rz                    # Receive files

# Options
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

## Comparison with lrzsz

| | rzsz | lrzsz |
|--|------|-------|
| Language | Rust | C |
| Binary size (static) | 449 KB | ~200 KB |
| Binary count | 1 (`zz`) | 2 (`lsz` + `lrz`) |
| Unified command | `zz` (auto-detect) | No |
| Memory safety | Compile-time | Manual |
| Terminal restore | RAII (guaranteed) | Signal handler (fragile) |
| Timeout mechanism | `poll()` | `alarm()`/SIGALRM |
| State management | Struct fields | 50+ global variables |
| Protocol state machine | Explicit enum + match | Implicit goto chains |
| CRC-16 bug (xstrdup) | N/A | Fixed in our fork |
| Path traversal protection | Default on | Opt-in |

### Performance (10MB pipe transfer)

| Sender | Receiver | Throughput |
|--------|----------|-----------|
| C lsz | C lrz | 116 MB/s |
| Rust sz | C lrz | 49 MB/s |
| C lsz | Rust rz | 45 MB/s |
| Rust sz | Rust rz | 29 MB/s |

> Rust version is 2-3x slower in pipe benchmarks due to per-byte escape encoding (vs C's batch `zsendline_s`). In real-world SSH/serial transfers, network latency dominates — the difference is imperceptible.

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
git clone https://github.com/nicholasching/rzsz.git
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

## License

GPL-2.0-or-later (same as lrzsz)

## Credits

- Original rzsz by Chuck Forsberg (Omen Technology)
- lrzsz maintained by Uwe Ohse
- ZModem protocol specification (1988)

---

<a id="中文"></a>

# rzsz — 经典文件传输工具的 Rust 重写

## 简介

rzsz 是经典 Unix 文件传输工具 lrzsz 的现代化 Rust 重写。通过 ZModem、XModem、YModem 协议在终端连接上传输文件。

## 特性

- **单一二进制** — 一个 `zz` 文件（449KB 静态链接），所有命令通过符号链接
- **直接替换** — 可作为 `rz`、`sz`、`rrz`、`rsz` 使用（argv[0] 自动检测）
- **智能模式** — `zz file` 发送，`zz` 接收，无需区分命令
- **ZModem** — CRC-16/32、自适应块大小、多文件批量传输、断点续传
- **XModem** — 128 字节/1K 字节块、CRC-16 和校验和模式
- **YModem** — 批量传输、文件头、文件大小跟踪
- **安全** — 默认受限模式、路径穿越防护、文件名净化
- **终端兼容** — 支持 Xshell、SecureCRT、iTerm2、MobaXterm
- **零依赖** — musl 静态链接，可在任何 Linux 上运行

## 快速开始

### 从 cargo 安装

```bash
cargo install rzsz
```

### 从二进制安装

从 [Releases](https://github.com/nicholasching/rzsz/releases) 下载，然后：

```bash
tar xzf zz-linux-x86_64-musl.tar.gz -C /usr/local/bin/
```

### 替换系统 rz/sz（可选）

```bash
# 备份旧版
sudo mv /usr/bin/rz /usr/bin/rz.old 2>/dev/null
sudo mv /usr/bin/sz /usr/bin/sz.old 2>/dev/null

# 链接新版
sudo ln -sf /usr/local/bin/zz /usr/bin/rz
sudo ln -sf /usr/local/bin/zz /usr/bin/sz
```

## 使用方法

```bash
# 统一命令
zz file1 file2        # 发送文件
zz                    # 接收文件

# 传统命令（zz 的符号链接）
sz file1 file2        # 发送文件
rz                    # 接收文件

# 常用选项
zz -r file            # 断点续传
zz -p                 # 接收时保护已有文件（不覆盖）
zz -E                 # 接收时已有文件自动改名（.1, .2, ...）
zz -e file            # 转义所有控制字符
zz -T file            # Turbo 模式（减少转义，更快）
zz -8 file            # 尝试 8K 块
zz -q file            # 静默模式
zz -v file            # 详细模式
zz --help             # 查看所有选项
```

## 文件覆盖策略

| 选项 | 行为 |
|------|------|
| 默认（无选项） | 覆盖已有文件 |
| `-p` / `--protect` | 跳过已有文件，显示提示 |
| `-E` / `--rename` | 自动改名（file.1, file.2, ...） |

## 从源码构建

```bash
git clone https://github.com/nicholasching/rzsz.git
cd rzsz
cargo build --release

# 静态编译（无运行时依赖）
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl

# 运行测试
cargo test
bash tests/interop.sh
```

## 发布到 crates.io

```bash
# 1. 注册账号并登录
cargo login

# 2. 检查包信息
cargo package --list

# 3. 试运行（不实际发布）
cargo publish --dry-run

# 4. 发布
cargo publish

# 5. 打 tag 触发 GitHub Actions 自动构建
git tag v0.1.0
git push origin v0.1.0
```

## 许可证

GPL-2.0-or-later（与 lrzsz 相同）
