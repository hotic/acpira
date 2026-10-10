"""Reading and writing the inventory file.

Format (version 2):

    {"version": 2, "items": {"SKU": {"name": "Widget", "lots": [{"qty": 5, "expires": null, "received": "2026-10-01"}]}}}

Version 1 files ({"items": {"SKU": {"name": ..., "qty": N}}}) are converted on load.
"""

import json
import os
import tempfile

VERSION = 2
EPOCH = "1970-01-01"


class StoreError(Exception):
    pass


def empty():
    return {"version": VERSION, "items": {}}


def migrate_v1(data):
    items = {}
    for sku, it in data.get("items", {}).items():
        qty = it.get("qty", 0)
        lots = [{"qty": qty, "expires": None, "received": EPOCH}] if qty > 0 else []
        items[sku] = {"name": it.get("name", sku), "lots": lots}
    return {"version": VERSION, "items": items}


def load_raw(path):
    """(data as stored, or None when the file is missing)."""
    if not os.path.exists(path):
        return None
    with open(path, encoding="utf-8") as f:
        try:
            return json.load(f)
        except json.JSONDecodeError as e:
            raise StoreError(f"corrupt inventory file: {e}") from None


def upgrade(data):
    if data is None:
        return empty()
    v = data.get("version")
    if v == 1:
        return migrate_v1(data)
    if v == VERSION:
        return data
    raise StoreError(f"unsupported version {v}")


def load(path):
    return upgrade(load_raw(path))


def save(path, data):
    # Write a temp file next to the target and rename it over, so a crash never leaves half a file
    d = os.path.dirname(os.path.abspath(path))
    fd, tmp = tempfile.mkstemp(dir=d, prefix=".inv-", suffix=".json")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2, sort_keys=True)
            f.write("\n")
        os.replace(tmp, path)
    except BaseException:
        os.unlink(tmp)
        raise
