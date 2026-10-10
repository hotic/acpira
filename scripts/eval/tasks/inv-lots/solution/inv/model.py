"""Stock operations on the loaded data (version 2, lots)."""


class InsufficientStock(Exception):
    def __init__(self, sku, have, need):
        super().__init__(f"insufficient stock for {sku}: have {have}, need {need}")


def expired(lot, today):
    return lot["expires"] is not None and lot["expires"] < today


class Inventory:
    def __init__(self, data, today):
        self.data = data
        self.today = today

    @property
    def items(self):
        return self.data["items"]

    def add(self, sku, qty, name=None, expires=None):
        item = self.items.setdefault(sku, {"name": name or sku, "lots": []})
        if name:
            item["name"] = name
        item["lots"].append({"qty": qty, "expires": expires, "received": self.today})

    def available(self, sku):
        lots = self.items.get(sku, {}).get("lots", [])
        return sum(l["qty"] for l in lots if not expired(l, self.today))

    def remove(self, sku, qty):
        have = self.available(sku)
        if have < qty:
            raise InsufficientStock(sku, have, qty)
        lots = self.items[sku]["lots"]
        # Earliest expiry first, lots without expiry last; sorted() keeps receiving order for ties
        order = sorted((i for i, l in enumerate(lots) if not expired(l, self.today)),
                       key=lambda i: (lots[i]["expires"] is None, lots[i]["expires"] or ""))
        left = qty
        for i in order:
            take = min(left, lots[i]["qty"])
            lots[i]["qty"] -= take
            left -= take
            if left == 0:
                break
        self.items[sku]["lots"] = [l for l in lots if l["qty"] > 0]

    def rows(self):
        """(sku, name, available qty) sorted by SKU."""
        return [(sku, it["name"], self.available(sku)) for sku, it in sorted(self.items.items())]

    def expiring(self, until):
        """(expires, sku, qty, expired) for lots expiring on or before `until`, in report order."""
        out = []
        for sku, it in self.items.items():
            for n, l in enumerate(it["lots"]):
                if l["expires"] is not None and l["expires"] <= until:
                    out.append((l["expires"], sku, n, l["qty"], expired(l, self.today)))
        out.sort()
        return [(e, sku, q, x) for e, sku, _, q, x in out]

    def purge(self):
        gone = [(e, sku, q) for e, sku, q, x in self.expiring(self.today) if x]
        for it in self.items.values():
            it["lots"] = [l for l in it["lots"] if not expired(l, self.today)]
        return gone
