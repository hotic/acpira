#!/usr/bin/env python3
"""The Android app's view tree through uiautomator, the twin of ui.py, plus the actions a test needs.

  aui.py [filter]              one node per line: id, text / description, centre @x,y (pixels), bounds
  aui.py tap <query>           tap the first node whose id, text or description matches (id exactly, else substring)
  aui.py wait <query> [secs]   poll until a node matches (default 30 s); exits 1 on timeout, printing the last tree
  aui.py type <text>           type into the focused field (adb input text; spaces kept)
  aui.py gone <query> [secs]   poll until no node matches

Ids are the Compose test tags (testTagsAsResourceId), e.g. composer.send, session:<id>, permission.allow.
"""
import re
import subprocess
import sys
import time
import xml.etree.ElementTree as ET

ADB = ["adb"]


def dump():
    # `uiautomator dump` writes a file; exec-out cat avoids a pull. It fails now and then while the UI animates
    for _ in range(5):
        out = subprocess.run(ADB + ["exec-out", "uiautomator dump /dev/tty"], capture_output=True, text=True).stdout
        xml = out[out.find("<?xml"):out.rfind(">") + 1] if "<?xml" in out else ""
        if xml:
            try:
                return ET.fromstring(xml)
            except ET.ParseError:
                pass
        time.sleep(0.3)
    raise SystemExit("uiautomator dump failed")


def nodes(root):
    for n in root.iter("node"):
        rid = n.get("resource-id", "")
        text = n.get("text", "")
        desc = n.get("content-desc", "")
        b = [int(v) for v in re.findall(r"-?\d+", n.get("bounds", ""))]
        if not (rid or text or desc) or len(b) != 4:
            continue
        yield {"id": rid, "text": text, "desc": desc, "x": (b[0] + b[2]) // 2, "y": (b[1] + b[3]) // 2, "bounds": b}


def match(n, q):
    return n["id"] == q or (q in n["text"]) or (q in n["desc"])


def line(n):
    label = " | ".join(v for v in (n["text"], n["desc"]) if v).replace("\n", " ")
    return f'{n["id"] or "-"}  {label[:80]!r}  @{n["x"]},{n["y"]}  {n["bounds"]}'


def find(q):
    root = dump()
    found = [n for n in nodes(root) if match(n, q)]
    # Exact ids first, then exact texts, so `composer.send` or "Connect" never lands on a text that merely contains it
    found.sort(key=lambda n: (n["id"] != q, n["text"] != q and n["desc"] != q))
    return found, root


def main():
    args = sys.argv[1:]
    cmd = args[0] if args else ""
    if cmd == "tap":
        found, _ = find(args[1])
        if not found:
            raise SystemExit(f"no node matches {args[1]!r}")
        n = found[0]
        subprocess.run(ADB + ["shell", "input", "tap", str(n["x"]), str(n["y"])], check=True)
        print(line(n))
    elif cmd in ("wait", "gone"):
        q, limit = args[1], float(args[2]) if len(args) > 2 else 30.0
        end = time.time() + limit
        while True:
            found, root = find(q)
            if (cmd == "wait") == bool(found):
                if found:
                    print(line(found[0]))
                return
            if time.time() > end:
                for n in nodes(root):
                    print(line(n), file=sys.stderr)
                raise SystemExit(f"timed out after {limit:.0f} s: {cmd} {q!r}")
            time.sleep(0.5)
    elif cmd == "type":
        # adb input text takes %s for a space and no shell metacharacters unescaped
        text = re.sub(r"([\\'\"`$&|;<>()*?!#~])", r"\\\1", args[1]).replace(" ", "%s")
        subprocess.run(ADB + ["shell", "input", "text", text], check=True)
    else:
        q = cmd
        for n in nodes(dump()):
            if not q or match(n, q) or q in n["id"]:
                print(line(n))


if __name__ == "__main__":
    main()
