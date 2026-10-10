"""Stock operations on the loaded data."""


class InsufficientStock(Exception):
    def __init__(self, sku, have, need):
        super().__init__(f"insufficient stock for {sku}: have {have}, need {need}")


class Inventory:
    def __init__(self, data):
        self.data = data

    @property
    def items(self):
        return self.data["items"]

    def add(self, sku, qty, name=None):
        item = self.items.setdefault(sku, {"name": name or sku, "qty": 0})
        if name:
            item["name"] = name
        item["qty"] += qty

    def remove(self, sku, qty):
        have = self.items.get(sku, {}).get("qty", 0)
        if have < qty:
            raise InsufficientStock(sku, have, qty)
        self.items[sku]["qty"] = have - qty

    def rows(self):
        """(sku, name, qty) sorted by SKU."""
        return [(sku, it["name"], it["qty"]) for sku, it in sorted(self.items.items())]
