"""Command line: python3 -m inv [--db PATH] COMMAND ...

Exit status: 0 on success, 1 on bad input or a broken file, 2 when there is not enough stock.
"""

import argparse
import os
import sys

from . import store
from .model import InsufficientStock, Inventory


class UsageError(Exception):
    pass


def positive_int(text):
    try:
        n = int(text)
    except ValueError:
        n = 0
    if n <= 0:
        raise UsageError("quantity must be a positive integer")
    return n


def build_parser():
    p = argparse.ArgumentParser(prog="inv")
    p.add_argument("--db", default=os.environ.get("INV_DB", "inventory.json"))
    sub = p.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("add")
    a.add_argument("sku")
    a.add_argument("qty")
    a.add_argument("--name")
    r = sub.add_parser("remove")
    r.add_argument("sku")
    r.add_argument("qty")
    sub.add_parser("list")
    return p


def main(argv):
    args = build_parser().parse_args(argv)
    try:
        data = store.load(args.db)
        inv = Inventory(data)
        if args.cmd == "add":
            inv.add(args.sku, positive_int(args.qty), args.name)
            store.save(args.db, data)
        elif args.cmd == "remove":
            inv.remove(args.sku, positive_int(args.qty))
            store.save(args.db, data)
        elif args.cmd == "list":
            for sku, name, qty in inv.rows():
                print(f"{sku}\t{name}\t{qty}")
    except InsufficientStock as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    except (UsageError, store.StoreError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0
