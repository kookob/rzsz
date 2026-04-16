# Publishing Guide / 发布指南

## 发布到 crates.io

### 前置条件

1. 注册 [crates.io](https://crates.io) 账号（用 GitHub 登录）
2. 获取 API token：crates.io → Account Settings → API Tokens → New Token

### 步骤

```bash
# 1. 登录（只需一次）
cargo login <your-api-token>

# 2. 确认包信息正确
cargo package --list

# 3. 试运行（检查能否打包，不实际发布）
cargo publish --dry-run

# 4. 正式发布
cargo publish

# 安装验证
cargo install rzsz
zz --version
```

### 发布新版本

```bash
# 1. 更新 Cargo.toml 中的 version
# version = "0.2.0"

# 2. 提交
git add Cargo.toml
git commit -m "Bump version to 0.2.0"

# 3. 打 tag
git tag v0.2.0

# 4. 推送（触发 GitHub Actions 自动构建 release）
git push origin main
git push origin v0.2.0

# 5. 发布到 crates.io
cargo publish
```

## 发布到 GitHub Releases

打 tag 后 GitHub Actions 自动构建并发布：

```bash
git tag v0.1.0
git push origin v0.1.0
```

Actions 会自动：
- 构建 4 个目标平台的二进制
- 打包（含所有符号链接）
- 创建 GitHub Release 并附加构建产物

### 构建产物

| 文件 | 目标 |
|------|------|
| `zz-linux-x86_64.tar.gz` | Linux x86_64 (glibc) |
| `zz-linux-x86_64-musl.tar.gz` | Linux x86_64 (static, 推荐) |
| `zz-linux-aarch64.tar.gz` | Linux ARM64 (glibc) |
| `zz-linux-aarch64-musl.tar.gz` | Linux ARM64 (static) |

每个包含：
```
zz          # 主二进制
rz → zz     # 接收（符号链接）
sz → zz     # 发送（符号链接）
rrz → zz    # 接收（Rust 风格）
rsz → zz    # 发送（Rust 风格）
rb/sb/...   # XModem/YModem 变体
```

## 用户安装方式

### 方式 1: cargo install（需要 Rust 工具链）

```bash
cargo install rzsz
# 二进制安装到 ~/.cargo/bin/zz
# 手动创建符号链接：
ln -s ~/.cargo/bin/zz ~/.cargo/bin/rz
ln -s ~/.cargo/bin/zz ~/.cargo/bin/sz
```

### 方式 2: 下载预编译二进制（推荐）

```bash
# x86_64
curl -LO https://github.com/nicholasching/rzsz/releases/latest/download/zz-linux-x86_64-musl.tar.gz
sudo tar xzf zz-linux-x86_64-musl.tar.gz -C /usr/local/bin/

# ARM64
curl -LO https://github.com/nicholasching/rzsz/releases/latest/download/zz-linux-aarch64-musl.tar.gz
sudo tar xzf zz-linux-aarch64-musl.tar.gz -C /usr/local/bin/
```

### 方式 3: 替换系统 rz/sz

```bash
# 备份
sudo mv /usr/bin/rz /usr/bin/rz.old 2>/dev/null
sudo mv /usr/bin/sz /usr/bin/sz.old 2>/dev/null

# 替换
sudo ln -sf /usr/local/bin/zz /usr/bin/rz
sudo ln -sf /usr/local/bin/zz /usr/bin/sz

# 回退
sudo mv /usr/bin/rz.old /usr/bin/rz
sudo mv /usr/bin/sz.old /usr/bin/sz
```
