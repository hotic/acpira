import sys

from . import CalcError, run


def main(argv):
    if len(argv) != 2:
        print("usage: python3 -m calc FILE", file=sys.stderr)
        return 2
    with open(argv[1], encoding="utf-8") as f:
        source = f.read()
    try:
        for line in run(source):
            print(line)
    except CalcError as e:
        print(e, file=sys.stderr)
        return 1
    return 0


sys.exit(main(sys.argv))
