# Publishing Guide / 发布指南

## Publish to crates.io

### Prerequisites

1. Register at [crates.io](https://crates.io) (GitHub login)
2. Get API token: crates.io → Account Settings → API Tokens → New Token

### Steps

```bash
# 1. Login (one-time)
cargo login <your-api-token>

# 2. Verify package contents
cargo package --list

# 3. Dry run (check without publishing)
cargo publish --dry-run

# 4. Publish
cargo publish
```

After publishing, users can install with:

```bash
cargo install rzsz
# Creates ~/.cargo/bin/zz
# Manually create symlinks:
ln -s ~/.cargo/bin/zz ~/.cargo/bin/rz
ln -s ~/.cargo/bin/zz ~/.cargo/bin/sz
```

## Publish to GitHub Releases

Pushing a tag triggers GitHub Actions to auto-build and release:

```bash
# 1. Update version in Cargo.toml
# 2. Commit and tag
git add Cargo.toml
git commit -m "Release v0.1.0"
git tag v0.1.0
git push origin main
git push origin v0.1.0

# 3. Publish to crates.io
cargo publish
```

### Release Artifacts

GitHub Actions builds 4 targets automatically:

| File | Target | Recommended |
|------|--------|-------------|
| `zz-linux-x86_64.tar.gz` | Linux x86_64 (glibc) | |
| `zz-linux-x86_64-musl.tar.gz` | Linux x86_64 (static) | Yes |
| `zz-linux-aarch64.tar.gz` | Linux ARM64 (glibc) | |
| `zz-linux-aarch64-musl.tar.gz` | Linux ARM64 (static) | Yes |

Each tarball contains:

```
zz          # main binary
rz → zz     # receive (symlink)
sz → zz     # send (symlink)
rrz → zz    # receive, Rust style
rsz → zz    # send, Rust style
rb/sb/...   # XModem/YModem variants
```

## User Installation Methods

### Method 1: cargo install

```bash
cargo install rzsz
```

### Method 2: Download binary (recommended)

```bash
# x86_64
curl -LO https://github.com/kookob/rzsz/releases/latest/download/zz-linux-x86_64-musl.tar.gz
sudo tar xzf zz-linux-x86_64-musl.tar.gz -C /usr/local/bin/

# ARM64
curl -LO https://github.com/kookob/rzsz/releases/latest/download/zz-linux-aarch64-musl.tar.gz
sudo tar xzf zz-linux-aarch64-musl.tar.gz -C /usr/local/bin/
```

### Method 3: One-line install script

```bash
curl -fsSL https://raw.githubusercontent.com/kookob/rzsz/main/install.sh | sudo bash
```

### Method 4: Replace system rz/sz

```bash
# Backup
sudo mv /usr/bin/rz /usr/bin/rz.old 2>/dev/null
sudo mv /usr/bin/sz /usr/bin/sz.old 2>/dev/null

# Replace
sudo ln -sf /usr/local/bin/zz /usr/bin/rz
sudo ln -sf /usr/local/bin/zz /usr/bin/sz

# Rollback
sudo mv /usr/bin/rz.old /usr/bin/rz
sudo mv /usr/bin/sz.old /usr/bin/sz
```

### Uninstall

```bash
curl -fsSL https://raw.githubusercontent.com/kookob/rzsz/main/install.sh | sudo bash -s -- --uninstall
# or manually:
sudo rm /usr/local/bin/{zz,rz,sz,rrz,rsz,rb,sb,rrb,rsb,rx,sx,rrx,rsx}
sudo rm /usr/bin/{zz,rz,sz} 2>/dev/null
```
