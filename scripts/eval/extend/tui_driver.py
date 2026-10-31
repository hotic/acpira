"""Drive a terminal program under a pseudo-terminal and read its screen (the extension race's OpenCode checks, tui.ts).

Run as `uv run --with pyte python tui_driver.py COLS ROWS CWD -- CMD ARGS...`. Commands arrive as JSON lines on stdin and
each gets one JSON line back on stdout:
  {"op": "keys", "data": "hello\\r"}          -> {"ok": true}
  {"op": "click", "x": 10, "y": 5}            -> {"ok": true}   (0-based cell; SGR mouse move, press, release)
  {"op": "screen"}                            -> {"lines": [...], "bg": [["1f1d1b", 4000], ...], "alive": true}
  {"op": "quit"}                              -> {"ok": true}, then the program is stopped
The screen is emulated with pyte, which keeps 24-bit colours as hex. A background-colour query (OSC 11) is answered
with black, so a program that asks before drawing does not wait for a terminal that is not there.
"""
import collections
import fcntl
import json
import os
import pty
import select
import signal
import struct
import sys
import termios
import time

import pyte

cols, rows, cwd = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
cmd = sys.argv[sys.argv.index("--") + 1:]

screen = pyte.Screen(cols, rows)
stream = pyte.ByteStream(screen)
pid, fd = pty.fork()
if pid == 0:
    os.chdir(cwd)
    os.execvp(cmd[0], cmd)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

alive = True
pending = b""


def pump(timeout):
    """Read whatever the program wrote within `timeout` seconds into the screen."""
    global alive, pending
    end = time.time() + timeout
    while alive:
        left = end - time.time()
        r, _, _ = select.select([fd], [], [], max(0.0, left))
        if fd not in r:
            if left <= 0:
                return
            continue
        try:
            data = os.read(fd, 65536)
        except OSError:
            alive = False
            return
        if not data:
            alive = False
            return
        pending = (pending + data)[-64:]
        if b"\x1b]11;?" in pending:
            os.write(fd, b"\x1b]11;rgb:0000/0000/0000\x1b\\")
            pending = b""
        stream.feed(data)


def reply(value):
    sys.stdout.write(json.dumps(value) + "\n")
    sys.stdout.flush()


stdin = sys.stdin.buffer
buf = b""
while True:
    r, _, _ = select.select([stdin, fd] if alive else [stdin], [], [], 0.05)
    if alive and fd in r:
        pump(0.0)
    if stdin not in r:
        continue
    chunk = os.read(stdin.fileno(), 65536)
    if not chunk:
        break
    buf += chunk
    while b"\n" in buf:
        line, buf = buf.split(b"\n", 1)
        if not line.strip():
            continue
        m = json.loads(line)
        op = m.get("op")
        if op == "keys":
            if alive:
                os.write(fd, m["data"].encode())
            pump(0.05)
            reply({"ok": True})
        elif op == "click":
            x, y = m["x"] + 1, m["y"] + 1
            if alive:
                # Some interfaces hit-test on the pointer's last motion, so move there before pressing
                os.write(fd, f"\x1b[<35;{x};{y}M".encode())
                pump(0.05)
                os.write(fd, f"\x1b[<0;{x};{y}M".encode())
                pump(0.05)
                os.write(fd, f"\x1b[<0;{x};{y}m".encode())
            pump(0.05)
            reply({"ok": True})
        elif op == "screen":
            pump(0.1)
            bg = collections.Counter(screen.buffer[y][x].bg for y in range(rows) for x in range(cols))
            reply({"lines": [l.rstrip() for l in screen.display], "bg": bg.most_common(4), "alive": alive})
        elif op == "quit":
            reply({"ok": True})
            buf = b""
            break
    else:
        continue
    break

try:
    os.kill(pid, signal.SIGTERM)
    time.sleep(0.5)
    os.kill(pid, signal.SIGKILL)
except ProcessLookupError:
    pass
