"""Reading and writing the inventory file.

Format (version 1):

    {"version": 1, "items": {"SKU": {"name": "Widget", "qty": 5}}}
"""

import json
import os
import tempfile

VERSION = 1


class StoreError(Exception):
    pass


def empty():
    return {"version": VERSION, "items": {}}


def load(path):
    if not os.path.exists(path):
        return empty()
    with open(path, encoding="utf-8") as f:
        try:
            data = json.load(f)
        except json.JSONDecodeError as e:
            raise StoreError(f"corrupt inventory file: {e}") from None
    if data.get("version") != VERSION:
        raise StoreError(f"unsupported version {data.get('version')}")
    return data


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
