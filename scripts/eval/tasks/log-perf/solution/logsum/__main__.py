import sys

from . import summarize

if len(sys.argv) != 2:
    print("usage: python3 -m logsum FILE", file=sys.stderr)
    sys.exit(2)
with open(sys.argv[1], encoding="utf-8", errors="replace") as f:
    sys.stdout.write(summarize(f))
