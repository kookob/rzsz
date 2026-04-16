# rzsz 项目总结

## 项目概述

rzsz 是经典 Unix 文件传输工具 lrzsz 的 Rust 完全重写。项目包含两个二进制程序 `rsz`（发送）和 `rrz`（接收），支持 ZModem、XModem、YModem 三种协议，通过 argv[0] 检测自动切换协议。

项目源于对 lrzsz（30 年历史的 C 代码）的现代化需求：终端显示乱码、全局变量泛滥、alarm/SIGALRM 超时机制脆弱、已知缓冲区溢出等问题。经过 Go vs Rust 技术对比后选择 Rust，主要因为二进制大小（嵌入式场景关键约束）、编译期内存安全、以及 enum+match 对协议状态机建模的天然优势。

## 双轨制执行

### 轨道一：lrzsz C 代码修复（`/ob/code/opensource/lrzsz/`）

| 阶段 | 内容 |
|------|------|
| 1. 显示乱码修复 | `zm.c` 的 `write(1,...)` 改为 `fwrite`+`fflush`；`zglobal.h` 的 `vchar`/`vstring` 宏加 `fflush(stderr)`；`canit.c`/`lsz.c`/`lrz.c` 信号处理函数输出顺序纠正 |
| 2. 已知 bug 修复 | `utils.c` 的 `xstrdup()` off-by-one 堆溢出修复；`lsz.c`/`lrz.c` 的 `--stop-at` 时分赋值修复 |
| 3. 遗留代码清理 | 41 个 `register` 关键字移除；72 个 gettext `_()` 包装移除；NLS/gettext 从构建系统移除；`configure.ac`/`Makefile.am` 清理 |
| 4. 全局变量整理 | 11 个帧状态全局变量（Rxtimeout、Rxhdr、Txhdr 等）整合到 `struct zm_frame_state`，通过 `#define` 宏保持向后兼容 |
| 5. alarm→select | `zreadline.c` 的 `alarm()`/SIGALRM 替换为 `select()` 超时；`siginterrupt()` 调用移除 |

### 轨道二：rzsz Rust 重写（`/ob/code/opensource/rzsz/`）

| 阶段 | 内容 |
|------|------|
| 6. 项目脚手架 | Cargo.toml、目录结构、rsz/rrz 入口、git 仓库初始化 |
| 7. I/O 抽象+CRC | CRC-16/CRC-32 查找表（从 C 精确移植）；`ProtocolWriter`/`StatusWriter` newtype 编译期隔离；`ModemReader` poll 超时缓冲读取（含 `unread_byte` 推回）；`TerminalGuard` Drop RAII 终端恢复 |
| 8. ZModem 协议核心 | `FrameType` 枚举（20 种帧类型）；`FrameEncoder`（hex/bin16/bin32 header + data16/data32）；`Session` 状态机（header 收发、ZDLE 转义解码、CRC 校验）；`EscapeTable` 动态转义表 |
| 9. 发送端+接收端 | `sender.rs`：`get_receiver_init`→`send_file`→`send_file_data`（迭代 ZRPOS 重同步、自适应块大小、多文件批量）；`receiver.rs`：`try_zmodem`→`receive_files`→`receive_file_data`（ZRINIT 能力协商、流式数据接收、batch 结束检测）|
| 10. XModem/YModem | `xmodem.rs`：128B/1K 块传输，CRC-16/checksum，`BlockResult` 枚举区分新数据/重复；`ymodem.rs`：批量传输 block 0 文件头，文件名 basename 安全提取 |
| 11. 互操作测试 | `tests/interop.sh`：11 个测试用例覆盖 rsz↔lrz、lsz↔rrz、rsz↔rrz，含 1MB 大文件和多文件 |
| 12. Codex 审查修复 | 15 个问题全部修复（8 High + 5 Medium + 2 Low），涵盖死锁消除、路径穿越防护、栈溢出修复、协议合规 |

## 关键技术决策

### 为什么选 Rust 而非 Go

| 维度 | Rust | Go |
|------|------|-----|
| 二进制大小 | 445 KB (musl static) | ~1.65 MB |
| 状态机建模 | enum + exhaustive match | int 常量 + switch（无穷举检查）|
| 内存安全 | 编译期保证 | 运行时边界检查 |
| 终端恢复 | Drop trait RAII | defer（os.Exit 不触发）|
| 超时机制 | nix::poll | context.Context（更简洁）|
| 交叉编译 | cross 工具 | 零配置（更简单）|

决定性因素：二进制大小（嵌入式/IoT 硬约束）和协议状态机建模。

### C→Rust 架构映射

| C (lrzsz) | Rust (rzsz) |
|-----------|-------------|
| 50+ 全局变量 | `Session` 结构体字段 |
| `zgethdr()` goto 状态机 | `Session::receive_header()` + `match FrameType` |
| `setjmp`/`longjmp` | `AtomicBool` 信号标志 |
| `alarm()`/SIGALRM | `nix::poll::poll()` |
| stdout/stderr 混用 | `ProtocolWriter`/`StatusWriter` newtype |
| `#define updcrc(cp,crc)` 宏 | `update_crc16()`/`update_crc32()` 函数 |
| `register` 关键字 | 不需要（编译器优化）|
| `_(text)` gettext | 不需要（直接字符串）|

### 关键 bug 发现与修复

| bug | 位置 | 影响 |
|-----|------|------|
| CRC-16 计算公式错误 | `crc.rs` | Rust 版 `table[(crc>>8)^byte]` 与 C 版 `table[(crc>>8)&255] ^ (crc<<8) ^ cp` 不等价，导致所有 hex header CRC 校验失败 |
| ZF0 字节序搞反 | `sender.rs` | ZRINIT 能力标志在 hdr[3] 不是 hdr[0]（ZF0=3），导致 CRC-32 未启用 |
| OwnedFd 关闭 stdin | `terminal.rs` | `TerminalGuard` 用 `OwnedFd` 包装 fd 0，drop 时会 close stdin 导致管道通信断裂 |
| ZRPOS 无限递归 | `sender.rs` | 远端持续发 ZRPOS 可栈溢出，改为 `'resync` 迭代循环 |
| YModem 路径穿越 | `ymodem.rs` | 远端发 `../../.ssh/authorized_keys` 可在任意位置写文件，改为 `Path::file_name()` basename 提取 |

## 项目统计

```
源文件：15 个 .rs 文件
代码量：3,277 行 Rust
测试：  10 单元测试 + 11 集成测试 = 21 测试全部通过
协议：  ZModem (CRC-16/32, 自适应块, 多文件批量)
        XModem (128B/1K, CRC-16/checksum)
        YModem (批量传输, 文件头, 文件名安全校验)
CLI：   rsz 12 个选项 + rrz 13 个选项
```

### 二进制大小

| 构建 | rsz | rrz |
|------|-----|-----|
| release (glibc) | 352 KB | 356 KB |
| release (musl static) | 445 KB | 441 KB |
| C 原版 lsz/lrz | ~197 KB | ~192 KB |

### 互操作测试矩阵

| | C lrz | Rust rrz |
|--|-------|----------|
| **Rust rsz** | PASS (text, 100KB, 1MB, multi) | PASS (text, 100KB, 1MB, empty) |
| **C lsz** | PASS (原生 fastcheck) | PASS (text, 100KB, 1MB) |

### Git 提交历史

```
320311c Fix all 15 Codex review issues (8 High, 5 Medium, 2 Low)
58e0281 Fix interop test paths, all 11 tests pass
8ff2862 Add interop test script and verify musl static build
a0bdacc Add XModem/YModem protocols and multi-file ZModem batch transfer
af5d8a8 Add CLI options, fix session end, pass 1MB interop tests
5233973 Fix CRC-16, ZF0 byte order, TerminalGuard fd ownership
a8daaae Implement ZModem sender and receiver core logic
5f0af0b Initial rzsz scaffold: Rust rewrite of lrzsz
```

## 目录结构

```
/ob/code/opensource/rzsz/
├── Cargo.toml              # 依赖：nix, signal-hook, log, env_logger
├── CLAUDE.md               # Claude Code 引导文件
├── SUMMARY.md              # 本文件
├── src/
│   ├── lib.rs
│   ├── sender.rs           # ZModem 发送（多文件、自适应块、迭代 ZRPOS）
│   ├── receiver.rs         # ZModem 接收（batch 结束检测、protect/clobber）
│   ├── xmodem.rs           # XModem 协议（BlockResult 枚举、双 CAN 取消）
│   ├── ymodem.rs           # YModem 协议（文件名安全、块号校验）
│   ├── bin/
│   │   ├── rsz.rs          # 发送端 CLI（12 选项、协议自动检测）
│   │   └── rrz.rs          # 接收端 CLI（13 选项、-R/-U restricted）
│   ├── zmodem/
│   │   ├── mod.rs
│   │   ├── frame.rs        # 帧编解码（FrameEncoder, FrameType 枚举）
│   │   ├── crc.rs          # CRC-16/CRC-32（10 个单元测试）
│   │   ├── escape.rs       # ZDLE 转义表（EscapeTable, Clone）
│   │   └── session.rs      # Session 状态机（迭代 read_escaped、unread_byte）
│   └── serial/
│       ├── mod.rs           # ProtocolWriter/StatusWriter newtype
│       ├── reader.rs        # ModemReader（poll 超时、unread_byte 推回）
│       └── terminal.rs      # TerminalGuard（BorrowedFd、Drop RAII）
└── tests/
    └── interop.sh           # 11 测试用例自动化脚本
```

## 后续可扩展方向

- ARM/MIPS musl 交叉编译验证
- turbo 模式和窗口流控实际接入协议逻辑
- ASCII 模式传输（CR/LF 转换）
- syslog 集成
- man page 生成
- 性能基准测试（与 C 版 throughput 对比）
