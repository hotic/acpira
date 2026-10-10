"""Command line: python3 -m inv [--db PATH] [--today YYYY-MM-DD] COMMAND ...

Exit status: 0 on success, 1 on bad input or a broken file, 2 when there is not enough stock.
"""

import argparse
import datetime
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


def parse_date(text):
    # Strictly YYYY-MM-DD (fromisoformat alone also takes 20261001)
    try:
        if len(text) == 10:
            return datetime.date.fromisoformat(text).isoformat()
    except ValueError:
        pass
    raise UsageError(f"bad date {text}")


def build_parser():
    p = argparse.ArgumentParser(prog="inv")
    p.add_argument("--db", default=os.environ.get("INV_DB", "inventory.json"))
    p.add_argument("--today")
    sub = p.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("add")
    a.add_argument("sku")
    a.add_argument("qty")
    a.add_argument("--name")
    a.add_argument("--expires")
    r = sub.add_parser("remove")
    r.add_argument("sku")
    r.add_argument("qty")
    sub.add_parser("list")
    e = sub.add_parser("expiring")
    e.add_argument("days")
    sub.add_parser("purge")
    sub.add_parser("migrate")
    return p


def main(argv):
    args = build_parser().parse_args(argv)
    try:
        today = parse_date(args.today) if args.today else datetime.date.today().isoformat()
        if args.cmd == "migrate":
            return migrate(args.db)
        data = store.load(args.db)
        inv = Inventory(data, today)
        if args.cmd == "add":
            expires = parse_date(args.expires) if args.expires else None
            inv.add(args.sku, positive_int(args.qty), args.name, expires)
            store.save(args.db, data)
        elif args.cmd == "remove":
            inv.remove(args.sku, positive_int(args.qty))
            store.save(args.db, data)
        elif args.cmd == "list":
            for sku, name, qty in inv.rows():
                print(f"{sku}\t{name}\t{qty}")
        elif args.cmd == "expiring":
            if not args.days.isdigit():
                raise UsageError("days must be a non-negative integer")
            until = (datetime.date.fromisoformat(today) + datetime.timedelta(days=int(args.days))).isoformat()
            for e, sku, q, x in inv.expiring(until):
                print(f"{e}\t{sku}\t{q}" + (" (expired)" if x else ""))
        elif args.cmd == "purge":
            gone = inv.purge()
            for e, sku, q in gone:
                print(f"purged {sku} {e} {q}")
            print(f"purged {len(gone)} lots")
            store.save(args.db, data)
    except InsufficientStock as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    except (UsageError, store.StoreError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


def migrate(path):
    raw = store.load_raw(path)
    if raw is not None and raw.get("version") == store.VERSION:
        print(f"already at version {store.VERSION}")
        return 0
    data = store.upgrade(raw)
    store.save(path, data)
    print(f"migrated {len(data['items'])} items")
    return 0
