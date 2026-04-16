# rzsz — 经典文件传输工具的 Rust 重写

**[English](README.md)**

## 简介

rzsz 是经典 Unix 文件传输工具 [lrzsz](https://ohse.de/uwe/software/lrzsz.html) 的现代化 Rust 重写。通过 ZModem、XModem、YModem 协议在终端连接上传输文件。

一个二进制，一个命令 `zz`，即可替代 `rz` + `sz`。

## 特性

- **单一二进制** — 一个 `zz` 文件（449KB 静态链接），所有命令通过符号链接实现
- **直接替换** — 可作为 `rz`、`sz`、`rrz`、`rsz` 使用（通过 argv[0] 自动检测）
- **智能模式** — `zz file` 发送，`zz` 接收，无需记忆两个命令
- **ZModem** — CRC-16/32、自适应块大小、多文件批量传输、断点续传
- **XModem** — 128 字节/1K 字节块、CRC-16 和校验和模式
- **YModem** — 批量传输、文件头、文件大小跟踪
- **安全** — 默认受限模式、路径穿越防护、文件名净化
- **终端兼容** — 支持 Xshell、SecureCRT、iTerm2、MobaXterm
- **零依赖** — musl 静态链接构建，可在任何 Linux 系统上运行

## 快速开始

### 方式一：cargo 安装

```bash
cargo install rzsz
```

### 方式二：下载预编译二进制（推荐）

从 [Releases](https://github.com/kookob/rzsz/releases) 下载对应平台的包：

```bash
# x86_64
curl -LO https://github.com/kookob/rzsz/releases/latest/download/zz-linux-x86_64-musl.tar.gz
sudo tar xzf zz-linux-x86_64-musl.tar.gz -C /usr/local/bin/

# ARM64
curl -LO https://github.com/kookob/rzsz/releases/latest/download/zz-linux-aarch64-musl.tar.gz
sudo tar xzf zz-linux-aarch64-musl.tar.gz -C /usr/local/bin/
```

### 替换系统 rz/sz（可选）

```bash
# 备份旧版
sudo mv /usr/bin/rz /usr/bin/rz.old 2>/dev/null
sudo mv /usr/bin/sz /usr/bin/sz.old 2>/dev/null

# 链接新版
sudo ln -sf /usr/local/bin/zz /usr/bin/rz
sudo ln -sf /usr/local/bin/zz /usr/bin/sz

# 如需回退
sudo mv /usr/bin/rz.old /usr/bin/rz
sudo mv /usr/bin/sz.old /usr/bin/sz
```

## 使用方法

### 基本使用

```bash
# 统一命令（自动检测模式）
zz file1 file2        # 有文件参数 → 发送
zz                    # 无文件参数 → 接收

# 传统命令（zz 的符号链接，行为一致）
sz file1 file2        # 发送文件
rz                    # 接收文件
```

### 常用选项

```bash
zz -r file            # 断点续传（从上次中断处继续）
zz -p                 # 接收时保护已有文件（不覆盖）
zz -E                 # 接收时已有文件自动改名（.1, .2, ...）
zz -e file            # 转义所有控制字符（某些网络环境需要）
zz -T file            # Turbo 模式（减少转义，提升速度）
zz -8 file            # 尝试 8K 块大小
zz -q file            # 静默模式（不显示进度）
zz -v file            # 详细模式
zz --help             # 查看所有选项
```

### 文件覆盖策略

| 选项 | 行为 | 适用场景 |
|------|------|----------|
| 默认（无选项） | 覆盖已有文件 | 日常使用、重新上传 |
| `-p` / `--protect` | 跳过已有文件，显示提示 | 避免误覆盖重要文件 |
| `-E` / `--rename` | 自动改名（file.1, file.2, ...） | 保留所有版本 |

### 命令名与模式对应

`zz` 通过启动时的命令名自动判断工作模式：

| 命令名 | 模式 | 协议 |
|--------|------|------|
| `zz file` | 发送 | ZModem（自动） |
| `zz` | 接收 | ZModem（自动） |
| `sz` / `rsz` / `lsz` | 强制发送 | ZModem |
| `rz` / `rrz` / `lrz` | 强制接收 | ZModem |
| `sb` / `rsb` | 发送 | YModem |
| `rb` / `rrb` | 接收 | YModem |
| `sx` / `rsx` | 发送 | XModem |
| `rx` / `rrx` | 接收 | XModem |

## 终端兼容性

| 终端 | ZModem 支持 | 状态 |
|------|-------------|------|
| Xshell | 内置 | 已测试 |
| SecureCRT | 内置 | 兼容 |
| iTerm2 | 可配置触发器 | 兼容 |
| MobaXterm | 内置 | 兼容 |
| Tabby | 插件 | 兼容 |
| Windows Terminal | 无 | 请用 scp |
| PuTTY | 无 | 请用 scp |

> ZModem 需要终端模拟器主动参与文件传输。Windows Terminal 和 PuTTY 不具备此能力，建议使用 scp/sftp 替代。

## 与 lrzsz 对比

| | rzsz | lrzsz |
|--|------|-------|
| 语言 | Rust | C |
| 二进制大小（静态） | 449 KB | ~200 KB |
| 二进制数量 | 1 个（`zz`） | 2 个（`lsz` + `lrz`） |
| 统一命令 | `zz`（自动检测） | 无 |
| 内存安全 | 编译期保证 | 手动管理 |
| 终端恢复 | RAII（所有退出路径保证恢复） | 信号处理（可能遗漏） |
| 超时机制 | `poll()` | `alarm()`/SIGALRM |
| 状态管理 | 结构体字段 | 50+ 全局变量 |
| 路径穿越防护 | 默认开启 | 需手动开启 |

### 性能（10MB 管道传输测试）

| 发送端 | 接收端 | 吞吐量 |
|--------|--------|--------|
| C lsz | C lrz | 116 MB/s |
| Rust sz | C lrz | 49 MB/s |
| C lsz | Rust rz | 45 MB/s |
| Rust sz | Rust rz | 29 MB/s |

> Rust 版在管道测试中慢 2-3 倍，主要因为逐字节转义编码（C 版有批量优化 `zsendline_s`）。实际通过 SSH/串口传输时网络延迟是瓶颈，体感无差别。

## 从源码构建

```bash
git clone https://github.com/kookob/rzsz.git
cd rzsz

# 开发构建
cargo build

# 发布构建
cargo build --release

# 静态编译（无运行时依赖，推荐部署用）
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl

# 运行测试
cargo test                # 单元测试
bash tests/interop.sh     # 互操作集成测试
```

## 项目架构

```
src/
├── bin/zz.rs           # 统一入口：通过 argv[0] 自动检测发送/接收模式
├── sender.rs           # ZModem 发送：握手、文件数据、自适应块大小
├── receiver.rs         # ZModem 接收：ZRINIT 协商、文件写入、断点续传
├── xmodem.rs           # XModem：128B/1K 块、CRC-16/校验和
├── ymodem.rs           # YModem：批量传输、文件头
├── zmodem/
│   ├── frame.rs        # 帧编解码（hex/bin16/bin32）
│   ├── session.rs      # 协议状态机、帧头解析
│   ├── crc.rs          # CRC-16 和 CRC-32 查找表
│   └── escape.rs       # ZDLE 转义表
└── serial/
    ├── mod.rs           # ProtocolWriter/StatusWriter（类型安全的 I/O 隔离）
    ├── reader.rs        # 带 poll() 超时的缓冲读取器
    └── terminal.rs      # TerminalGuard（RAII 终端模式恢复）
```

## 许可证

GPL-2.0-or-later（与 lrzsz 相同）

## 致谢

- 原始 rzsz — Chuck Forsberg (Omen Technology)
- lrzsz 维护者 — Uwe Ohse
- ZModem 协议规范 (1988)
