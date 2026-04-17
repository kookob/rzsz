#!/bin/bash
#
# rzsz installer — install or upgrade rzsz (Rust rewrite of lrzsz)
# https://github.com/kookob/rzsz
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/kookob/rzsz/main/install.sh | bash
#   # or
#   bash install.sh [--uninstall]

set -euo pipefail

REPO="kookob/rzsz"
INSTALL_DIR="/usr/local/bin"
BIN_DIR="/usr/bin"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

info()  { echo -e "${GREEN}[INFO]${NC} $*"; }
warn()  { echo -e "${YELLOW}[WARN]${NC} $*"; }
error() { echo -e "${RED}[ERROR]${NC} $*"; }
ask()   { echo -en "${CYAN}[?]${NC} $* "; }

# Detect architecture
detect_arch() {
    local arch
    arch=$(uname -m)
    case "$arch" in
        x86_64|amd64)   echo "x86_64" ;;
        aarch64|arm64)   echo "aarch64" ;;
        *)               error "Unsupported architecture: $arch"; exit 1 ;;
    esac
}

# Detect if running as root
check_root() {
    if [ "$(id -u)" -ne 0 ]; then
        error "This script requires root privileges. Run with: sudo bash install.sh"
        exit 1
    fi
}

# Check if old lrzsz is installed
detect_old_lrzsz() {
    local found=0
    if command -v rz &>/dev/null; then
        local rz_path
        rz_path=$(command -v rz)
        local rz_version
        rz_version=$(rz --version 2>&1 | head -1 || true)
        if echo "$rz_version" | grep -qi "lrzsz\|GNU"; then
            echo "$rz_path"
            return 0
        fi
        # Check if it's already our version
        if echo "$rz_version" | grep -qi "rrz\|rzsz\|0\.1"; then
            echo "OURS:$rz_path"
            return 0
        fi
    fi
    echo ""
    return 1
}

# Uninstall rzsz
do_uninstall() {
    info "Uninstalling rzsz..."

    local links="zz rz sz rrz rsz rb sb rrb rsb rx sx rrx rsx"

    # Remove from /usr/bin (symlinks we created)
    for name in $links; do
        if [ -L "$BIN_DIR/$name" ]; then
            local target
            target=$(readlink -f "$BIN_DIR/$name" 2>/dev/null || true)
            if echo "$target" | grep -q "$INSTALL_DIR"; then
                rm -f "$BIN_DIR/$name"
                info "  Removed $BIN_DIR/$name"
            fi
        fi
    done

    # Remove from install dir
    for name in $links; do
        rm -f "$INSTALL_DIR/$name"
    done

    # Restore old lrzsz if backup exists
    if [ -f "$BIN_DIR/rz.old" ]; then
        ask "Restore old lrzsz (rz.old/sz.old)? [Y/n]"
        read -r ans < /dev/tty
        if [ "$ans" != "n" ] && [ "$ans" != "N" ]; then
            mv "$BIN_DIR/rz.old" "$BIN_DIR/rz" 2>/dev/null && info "  Restored $BIN_DIR/rz"
            mv "$BIN_DIR/sz.old" "$BIN_DIR/sz" 2>/dev/null && info "  Restored $BIN_DIR/sz"
        fi
    fi

    info "rzsz uninstalled."
    exit 0
}

# Main install
do_install() {
    local arch
    arch=$(detect_arch)
    info "Detected architecture: $arch"

    # Check for old lrzsz
    local old_rz
    old_rz=$(detect_old_lrzsz || true)

    if [ -n "$old_rz" ]; then
        if echo "$old_rz" | grep -q "^OURS:"; then
            info "rzsz is already installed at ${old_rz#OURS:}"
            ask "Reinstall/upgrade? [Y/n]"
            read -r ans < /dev/tty
            if [ "$ans" = "n" ] || [ "$ans" = "N" ]; then
                info "Cancelled."
                exit 0
            fi
        else
            warn "Old lrzsz detected: $old_rz"
            local old_version
            old_version=$(rz --version 2>&1 | head -1 || echo "unknown")
            warn "  Version: $old_version"
            echo ""
            ask "Remove old lrzsz and replace with rzsz? [Y/n]"
            read -r ans < /dev/tty
            if [ "$ans" = "n" ] || [ "$ans" = "N" ]; then
                info "Keeping old lrzsz. Installing rzsz as 'zz' only."
                info "(You can use 'zz' command, old rz/sz stay unchanged)"
            else
                # Backup old binaries
                for name in rz sz rb sb rx sx; do
                    local path="$BIN_DIR/$name"
                    if [ -f "$path" ] || [ -L "$path" ]; then
                        mv "$path" "${path}.old" 2>/dev/null || true
                        info "  Backed up $path → ${path}.old"
                    fi
                done
                # Try to remove lrzsz package
                if command -v apt-get &>/dev/null; then
                    ask "Also remove lrzsz system package (apt)? [y/N]"
                    read -r ans < /dev/tty
                    if [ "$ans" = "y" ] || [ "$ans" = "Y" ]; then
                        apt-get remove -y lrzsz 2>/dev/null && info "  Removed lrzsz package" || true
                    fi
                elif command -v yum &>/dev/null; then
                    ask "Also remove lrzsz system package (yum)? [y/N]"
                    read -r ans < /dev/tty
                    if [ "$ans" = "y" ] || [ "$ans" = "Y" ]; then
                        yum remove -y lrzsz 2>/dev/null && info "  Removed lrzsz package" || true
                    fi
                fi
            fi
        fi
    fi

    echo ""
    info "Downloading rzsz for $arch..."

    local url="https://github.com/$REPO/releases/latest/download/zz-linux-${arch}-musl.tar.gz"
    local tmpdir
    tmpdir=$(mktemp -d)
    trap "rm -rf $tmpdir" EXIT

    if command -v curl &>/dev/null; then
        curl -fsSL "$url" -o "$tmpdir/rzsz.tar.gz"
    elif command -v wget &>/dev/null; then
        wget -q "$url" -O "$tmpdir/rzsz.tar.gz"
    else
        error "Neither curl nor wget found. Install one and retry."
        exit 1
    fi

    info "Installing to $INSTALL_DIR..."
    tar xzf "$tmpdir/rzsz.tar.gz" -C "$INSTALL_DIR/"
    chmod +x "$INSTALL_DIR/zz"

    # Create system symlinks for rz/sz if they don't exist or were backed up
    for name in rz sz; do
        if [ ! -e "$BIN_DIR/$name" ]; then
            ln -sf "$INSTALL_DIR/$name" "$BIN_DIR/$name"
            info "  Linked $BIN_DIR/$name → $INSTALL_DIR/$name → zz"
        fi
    done

    # Also link zz to /usr/bin if not there
    if [ ! -e "$BIN_DIR/zz" ]; then
        ln -sf "$INSTALL_DIR/zz" "$BIN_DIR/zz"
    fi

    echo ""
    info "Installation complete!"
    echo ""
    echo "  Installed binaries:"
    echo "    $INSTALL_DIR/zz          (main binary, $(ls -lh $INSTALL_DIR/zz | awk '{print $5}'))"
    echo "    $INSTALL_DIR/rz → zz     (receive)"
    echo "    $INSTALL_DIR/sz → zz     (send)"
    echo "    $INSTALL_DIR/rrz → zz    (receive, Rust style)"
    echo "    $INSTALL_DIR/rsz → zz    (send, Rust style)"
    echo ""
    echo "  System commands:"
    local rz_target sz_target zz_target
    rz_target=$(readlink -f "$BIN_DIR/rz" 2>/dev/null || echo "not linked")
    sz_target=$(readlink -f "$BIN_DIR/sz" 2>/dev/null || echo "not linked")
    zz_target=$(readlink -f "$BIN_DIR/zz" 2>/dev/null || echo "not linked")
    echo "    rz → $rz_target"
    echo "    sz → $sz_target"
    echo "    zz → $zz_target"
    echo ""
    echo "  Version: $(zz --version 2>&1)"
    echo ""
    echo "  Usage:"
    echo "    zz file1 file2    # Send files"
    echo "    zz                # Receive files"
    echo "    sz file            # Send (traditional)"
    echo "    rz                 # Receive (traditional)"
    echo ""
    echo "  Uninstall:"
    echo "    sudo bash install.sh --uninstall"
}

# Entry point
main() {
    echo ""
    echo "  ┌─────────────────────────────────┐"
    echo "  │  rzsz installer                  │"
    echo "  │  Modern rz/sz in Rust            │"
    echo "  │  https://github.com/$REPO  │"
    echo "  └─────────────────────────────────┘"
    echo ""

    check_root

    if [ "${1:-}" = "--uninstall" ] || [ "${1:-}" = "uninstall" ]; then
        do_uninstall
    else
        do_install
    fi
}

main "$@"
