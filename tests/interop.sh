#!/usr/bin/env bash
#
# interop.sh -- integration / interoperability tests for rzsz
#
# Tests rsz/rrz (Rust) against lsz/lrz (C) and against themselves,
# transferring files of various sizes through a named-pipe pair.
#
# Usage:
#   ./tests/interop.sh [RSZ] [RRZ] [LSZ] [LRZ]
#
# All arguments are optional; sensible defaults are used.

set -euo pipefail

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

RSZ="$(realpath "${1:-$PROJECT_DIR/target/release/rsz}" 2>/dev/null || echo "${1:-$PROJECT_DIR/target/release/rsz}")"
RRZ="$(realpath "${2:-$PROJECT_DIR/target/release/rrz}" 2>/dev/null || echo "${2:-$PROJECT_DIR/target/release/rrz}")"
LSZ="$(realpath "${3:-/ob/code/opensource/lrzsz/src/lsz}" 2>/dev/null || echo "${3:-/ob/code/opensource/lrzsz/src/lsz}")"
LRZ="$(realpath "${4:-/ob/code/opensource/lrzsz/src/lrz}" 2>/dev/null || echo "${4:-/ob/code/opensource/lrzsz/src/lrz}")"

# Timeouts (seconds)
TIMEOUT_SMALL=15
TIMEOUT_LARGE=30

# ---------------------------------------------------------------------------
# Verify binaries exist
# ---------------------------------------------------------------------------
for bin in "$RSZ" "$RRZ" "$LSZ" "$LRZ"; do
    if [[ ! -x "$bin" ]]; then
        echo "ERROR: binary not found or not executable: $bin" >&2
        exit 1
    fi
done

# ---------------------------------------------------------------------------
# Temp directory + cleanup
# ---------------------------------------------------------------------------
TMPDIR_BASE="$(mktemp -d /tmp/rzsz-interop.XXXXXX)"
trap 'rm -rf "$TMPDIR_BASE"' EXIT

SRCDIR="$TMPDIR_BASE/src"
mkdir -p "$SRCDIR"

# ---------------------------------------------------------------------------
# Create test files
# ---------------------------------------------------------------------------
printf 'Hello, ZModem world!!!\n\n' > "$SRCDIR/small.txt"       # 24 bytes
dd if=/dev/urandom of="$SRCDIR/medium.bin" bs=1024 count=100 2>/dev/null  # 100 KB
dd if=/dev/urandom of="$SRCDIR/large.bin"  bs=1024 count=1024 2>/dev/null # 1 MB
touch "$SRCDIR/empty.dat"                                                  # 0 bytes

# Verify sizes
small_size=$(stat -c%s "$SRCDIR/small.txt")
medium_size=$(stat -c%s "$SRCDIR/medium.bin")
large_size=$(stat -c%s "$SRCDIR/large.bin")
empty_size=$(stat -c%s "$SRCDIR/empty.dat")

echo "Test files created:"
echo "  small.txt   : $small_size bytes"
echo "  medium.bin  : $medium_size bytes"
echo "  large.bin   : $large_size bytes"
echo "  empty.dat   : $empty_size bytes"
echo ""

# ---------------------------------------------------------------------------
# Counters
# ---------------------------------------------------------------------------
PASSED=0
FAILED=0
ERRORS=""

# ---------------------------------------------------------------------------
# run_test  test_name  sender_bin  sender_args  receiver_bin  receiver_args
#           timeout  src_file...
#
# Runs a ZModem file transfer through a pair of named pipes and compares
# the received files with the originals.
#
# Pipe wiring (bidirectional channel via two FIFOs):
#   sender:   stdin <- PIPE_A   stdout -> PIPE_B
#   receiver: stdin <- PIPE_B   stdout -> PIPE_A
#
# To avoid FIFO-open deadlock, we pre-open both pipes read-write on file
# descriptors in the main shell, then pass them to subprocesses.
# ---------------------------------------------------------------------------
run_test() {
    local test_name="$1"; shift
    local sender_bin="$1"; shift
    local sender_args="$1"; shift
    local receiver_bin="$1"; shift
    local receiver_args="$1"; shift
    local tout="$1"; shift
    local src_files=("$@")

    local recv_dir="$TMPDIR_BASE/recv_$$_${RANDOM}"
    mkdir -p "$recv_dir"

    printf "  %-50s " "$test_name"

    # Create fresh pipes for this test (reuse can leave stale data)
    local pipe_a="$TMPDIR_BASE/pa_$$_${RANDOM}"
    local pipe_b="$TMPDIR_BASE/pb_$$_${RANDOM}"
    mkfifo "$pipe_a" "$pipe_b"

    # Pre-open both pipes read-write to avoid deadlock.
    # O_RDWR on a FIFO does not block (POSIX).
    exec 7<>"$pipe_a"
    exec 8<>"$pipe_b"

    # Launch receiver in recv_dir (reads from pipe_b, writes to pipe_a)
    (cd "$recv_dir" && timeout "$tout" "$receiver_bin" $receiver_args) \
        <&8 >&7 2>/dev/null &
    local recv_pid=$!

    # Launch sender (reads from pipe_a, writes to pipe_b)
    timeout "$tout" "$sender_bin" $sender_args "${src_files[@]}" \
        <&7 >&8 2>/dev/null &
    local send_pid=$!

    # Close our copies of the fds so EOF propagates when processes exit
    exec 7>&-
    exec 8>&-

    local send_rc=0 recv_rc=0
    wait "$send_pid" 2>/dev/null || send_rc=$?
    wait "$recv_pid" 2>/dev/null || recv_rc=$?

    # Clean up pipe files
    rm -f "$pipe_a" "$pipe_b"

    # Compare files
    local all_match=true
    for src in "${src_files[@]}"; do
        local base
        base="$(basename "$src")"
        local received="$recv_dir/$base"

        if [[ ! -f "$received" ]]; then
            all_match=false
            break
        fi

        if ! cmp -s "$src" "$received"; then
            all_match=false
            break
        fi
    done

    if $all_match; then
        echo "PASS"
        PASSED=$((PASSED + 1))
    else
        echo "FAIL"
        FAILED=$((FAILED + 1))
        ERRORS="${ERRORS}  FAIL: ${test_name}\n"
    fi

    # Cleanup recv dir
    rm -rf "$recv_dir"
}

# ---------------------------------------------------------------------------
# Test cases
# ---------------------------------------------------------------------------
echo "=== rsz -> lrz ==="
run_test "rsz->lrz small text"    "$RSZ" "-q" "$LRZ" "-q -y" "$TIMEOUT_SMALL" "$SRCDIR/small.txt"
run_test "rsz->lrz medium binary" "$RSZ" "-q" "$LRZ" "-q -y" "$TIMEOUT_SMALL" "$SRCDIR/medium.bin"
run_test "rsz->lrz large binary"  "$RSZ" "-q" "$LRZ" "-q -y" "$TIMEOUT_LARGE" "$SRCDIR/large.bin"
echo ""

echo "=== lsz -> rrz ==="
run_test "lsz->rrz small text"    "$LSZ" "-q" "$RRZ" "-q -y" "$TIMEOUT_SMALL" "$SRCDIR/small.txt"
run_test "lsz->rrz medium binary" "$LSZ" "-q" "$RRZ" "-q -y" "$TIMEOUT_SMALL" "$SRCDIR/medium.bin"
run_test "lsz->rrz large binary"  "$LSZ" "-q" "$RRZ" "-q -y" "$TIMEOUT_LARGE" "$SRCDIR/large.bin"
echo ""

echo "=== rsz -> rrz ==="
run_test "rsz->rrz small text"    "$RSZ" "-q" "$RRZ" "-q -y" "$TIMEOUT_SMALL" "$SRCDIR/small.txt"
run_test "rsz->rrz medium binary" "$RSZ" "-q" "$RRZ" "-q -y" "$TIMEOUT_SMALL" "$SRCDIR/medium.bin"
run_test "rsz->rrz large binary"  "$RSZ" "-q" "$RRZ" "-q -y" "$TIMEOUT_LARGE" "$SRCDIR/large.bin"
run_test "rsz->rrz empty file"    "$RSZ" "-q" "$RRZ" "-q -y" "$TIMEOUT_SMALL" "$SRCDIR/empty.dat"
echo ""

echo "=== rsz -> lrz multi-file ==="
run_test "rsz->lrz multi (small+medium)" "$RSZ" "-q" "$LRZ" "-q -y" "$TIMEOUT_LARGE" \
    "$SRCDIR/small.txt" "$SRCDIR/medium.bin"
echo ""

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
TOTAL=$((PASSED + FAILED))
echo "=============================="
echo "Results: $PASSED passed, $FAILED failed (out of $TOTAL)"
echo "=============================="

if [[ $FAILED -gt 0 ]]; then
    echo ""
    echo "Failed tests:"
    echo -e "$ERRORS"
    exit 1
fi

exit 0
