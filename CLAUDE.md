# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目概述

rzsz 是 lrzsz（X/Y/ZModem 文件传输工具）的 Rust 重写版本。产出两个二进制：`rsz`（发送）和 `rrz`（接收），通过 argv[0] 检测支持 XModem（rsx/rrx）和 YModem（rsb/rrb）。

原 C 版本在 `/ob/code/opensource/lrzsz/`，协议规范在 `lrzsz/doc/zmodem-1988-10-14.txt`。

## 构建和测试

```bash
cargo build                    # 开发构建
cargo build --release          # 优化构建（opt-level=s, LTO, strip, panic=abort）
cargo test                     # 运行所有单元测试
cargo test --test interop      # 互操作测试（与 C 版 lrzsz）
```

Release 二进制目标 < 500KB（musl static）。

## 架构

**核心设计原则：零全局变量，显式状态机，类型级 I/O 隔离。**

```
src/
├── lib.rs                    # 库入口
├── bin/
│   ├── rsz.rs                # 发送端（argv[0] 检测协议）
│   └── rrz.rs                # 接收端
├── zmodem/
│   ├── frame.rs              # 帧编解码（FrameEncoder）、协议常量、FrameType 枚举（20 种）
│   ├── crc.rs                # CRC-16/CRC-32 查找表和函数
│   ├── escape.rs             # ZDLE 转义表（EscapeTable）
│   └── session.rs            # Session 状态机、ZError、帧收发逻辑
└── serial/
    ├── mod.rs                # ProtocolWriter/StatusWriter newtype（编译期隔离 stdout/stderr）
    ├── reader.rs             # ModemReader：poll() 超时缓冲读取（替代 alarm/SIGALRM）
    └── terminal.rs           # TerminalGuard：RAII 终端模式管理（Drop 自动恢复）
```

**关键类型映射（C → Rust）：**
- 50+ 全局变量 → `Session` 结构体字段
- `zgethdr()` goto 状态机 → `Session::receive_header()` + `match FrameType`
- `setjmp/longjmp` → `AtomicBool` 信号标志 + `signal_hook`
- `alarm()/SIGALRM` → `nix::poll::poll()` 超时
- stdout/stderr 混合 → `ProtocolWriter`/`StatusWriter` newtype

## 依赖

- `nix` — termios、poll、signal（POSIX 系统调用）
- `signal-hook` — 安全信号处理
- `log` + `env_logger` — 日志
