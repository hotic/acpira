# Lots and expiry dates

Stock arrives in lots, and perishable lots expire. Track each lot instead of one quantity per item.

## Data format, version 2

```json
{"version": 2, "items": {"MILK": {"name": "Milk", "lots": [
  {"qty": 5, "expires": "2026-12-01", "received": "2026-10-01"},
  {"qty": 3, "expires": null, "received": "2026-10-02"}
]}}}
```

- `lots` are kept in the order they were received. `expires` is `null` for stock that does not expire.
- Dates are `YYYY-MM-DD`. A lot that reaches quantity 0 is removed from the list; an item whose lots are all gone stays
  with an empty list.

## Migration from version 1

- Every command reads version 1 files. Each version 1 item becomes one lot `{"qty": QTY, "expires": null, "received":
  "1970-01-01"}`, and an item with quantity 0 gets an empty `lots` list. Names are kept.
- A command that changes stock writes the file back as version 2. `list` and `expiring` do not write.
- New command `migrate`: rewrites the file as version 2 and prints `migrated N items`; on a file that is already
  version 2 it prints `already at version 2` and leaves it untouched. On a missing file it prints `migrated 0 items`
  and writes an empty version 2 file.
- Any other version: exit status 1 with `error: unsupported version X`.

## Commands

- Global option `--today YYYY-MM-DD` sets the current date (default: the real date).
- `add SKU QTY [--name NAME] [--expires YYYY-MM-DD]` appends a lot received today. The name works as before (defaults
  to the SKU for a new item, `--name` renames).
- A lot is **expired** when its expiry date is before today; a lot expiring today is still good.
- `remove SKU QTY` takes stock from the lots that are not expired: the earliest expiry date first, lots without expiry
  last, ties in receiving order. Not enough unexpired stock: exit status 2, `error: insufficient stock for SKU: have H,
  need N` (H counts unexpired stock only), nothing changes.
- `list` keeps its format `SKU<TAB>NAME<TAB>QTY`; QTY counts unexpired stock only.
- New command `expiring DAYS` (DAYS a non-negative integer): every lot with an expiry date on or before today + DAYS,
  expired ones included, one per line as `EXPIRES<TAB>SKU<TAB>QTY`, with ` (expired)` appended for expired lots. Sorted
  by expiry date, then SKU, then receiving order. Prints nothing when there are none.
- New command `purge`: removes every expired lot, printing `purged SKU EXPIRES QTY` for each, in the same order as
  `expiring`; then `purged N lots`.
- A bad date (in `--today` or `--expires`) is exit status 1 with `error: bad date TEXT`. The existing checks and exit
  statuses stay as they are.

Update the existing tests and add new ones.
