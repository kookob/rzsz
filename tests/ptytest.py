#!/usr/bin/env python3
"""Terminal-emulator style test: remote rz/sz on a pty, "terminal" on pipes.

Unlike interop.sh (FIFOs), this puts the remote side on a real pty slave and
pumps bytes between the pty master and the terminal program, like Xshell /
SecureCRT do. CORRUPT=N flips one byte every N bytes of the data stream
(after the first 64KB) to exercise ZRPOS resync on both sides.

usage: ptytest.py up|down REMOTE_BIN TERM_BIN FILE [timeout]
  up   : remote runs REMOTE_BIN (rz), terminal runs TERM_BIN FILE (sz)
  down : remote runs REMOTE_BIN FILE (sz), terminal runs TERM_BIN (rz)
examples:
  python3 tests/ptytest.py up   rz  /path/to/lsz big.bin
  CORRUPT=3000000 python3 tests/ptytest.py down sz /path/to/lrz big.bin
"""
import fcntl, hashlib, os, pty, select, shutil, subprocess, sys, tempfile, threading, time

mode, remote_bin, term_bin, path = sys.argv[1:5]
tout = float(sys.argv[5]) if len(sys.argv) > 5 else 60
path = os.path.abspath(path)
name = os.path.basename(path)
work = tempfile.mkdtemp(prefix="ptytest.")
recv_dir = os.path.join(work, "recv")
os.mkdir(recv_dir)

master, slave = pty.openpty()
if mode == "up":
    remote_cmd, remote_cwd, term_cmd, term_cwd = [remote_bin], recv_dir, [term_bin, path], work
else:
    remote_cmd, remote_cwd, term_cmd, term_cwd = [remote_bin, path], work, [term_bin], recv_dir

remote = subprocess.Popen(remote_cmd, stdin=slave, stdout=slave, stderr=slave, cwd=remote_cwd,
                          start_new_session=True,
                          preexec_fn=lambda: fcntl.ioctl(0, 0x540E, 0))  # TIOCSCTTY
os.close(slave)
term_err = open(os.path.join(work, "term.err"), "wb")
term = subprocess.Popen(term_cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                        stderr=term_err, cwd=term_cwd, bufsize=0)

stats = {"m2t": 0, "t2m": 0, "corrupted": 0}
stop = False
CORRUPT = int(os.environ.get("CORRUPT", "0"))
data_dir = "t2m" if mode == "up" else "m2t"


def maybe_corrupt(data, direction):
    if not CORRUPT or direction != data_dir:
        return data
    seen = stats[direction] - len(data)
    if seen < 65536:
        return data
    data = bytearray(data)
    for i in range(len(data)):
        if (seen + i) % CORRUPT == 0:
            data[i] ^= 0x55
            stats["corrupted"] += 1
    return bytes(data)


def master_to_term():
    while not stop:
        if not select.select([master], [], [], 0.2)[0]:
            continue
        try:
            data = os.read(master, 65536)
        except OSError:
            break
        if not data:
            break
        stats["m2t"] += len(data)
        try:
            term.stdin.write(maybe_corrupt(data, "m2t"))
        except (BrokenPipeError, ValueError):
            break


def term_to_master():
    while not stop:
        data = term.stdout.read(65536)
        if not data:
            break
        stats["t2m"] += len(data)
        os.write(master, maybe_corrupt(data, "t2m"))


threading.Thread(target=master_to_term, daemon=True).start()
threading.Thread(target=term_to_master, daemon=True).start()

t0 = time.time()
try:
    term.wait(timeout=tout)
    remote.wait(timeout=5)
except subprocess.TimeoutExpired:
    print("TIMEOUT: term rc=%s remote rc=%s" % (term.poll(), remote.poll()))
    term.kill()
    remote.kill()
stop = True
dt = time.time() - t0


def sha(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


got = os.path.join(recv_dir, name)
print("mode=%s remote=%s term=%s file=%r size=%d" % (
    mode, os.path.basename(remote_bin), os.path.basename(term_bin),
    name.encode("utf-8", "surrogateescape"), os.path.getsize(path)))
print("time=%.2fs remote->term=%d term->remote=%d corrupted=%d rc term=%s remote=%s" % (
    dt, stats["m2t"], stats["t2m"], stats["corrupted"], term.returncode, remote.returncode))
if os.path.exists(got):
    ok = sha(got) == sha(path)
    print("RESULT:", "MATCH" if ok else "MISMATCH size=%d" % os.path.getsize(got))
else:
    ok = False
    print("RESULT: MISSING")
term_err.close()
shutil.rmtree(work)
sys.exit(0 if ok else 1)
