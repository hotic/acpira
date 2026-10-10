import json
import os
import subprocess
import sys
import tempfile
import unittest

T = "2026-10-11"


class Base(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.db = os.path.join(self.dir.name, "inv.json")

    def tearDown(self):
        self.dir.cleanup()

    def inv(self, *args, today=T):
        extra = ["--today", today] if today else []
        return subprocess.run([sys.executable, "-m", "inv", "--db", self.db, *extra, *args], capture_output=True, text=True)

    def ok(self, *args, today=T):
        r = self.inv(*args, today=today)
        self.assertEqual(r.returncode, 0, (args, r.stderr))
        return r.stdout

    def data(self):
        with open(self.db, encoding="utf-8") as f:
            return json.load(f)

    def write(self, obj):
        with open(self.db, "w", encoding="utf-8") as f:
            json.dump(obj, f)


class Lots(Base):
    def test_add_creates_lots(self):
        self.ok("add", "MILK", "5", "--name", "Milk", "--expires", "2026-12-01")
        self.ok("add", "MILK", "3", today="2026-10-12")
        self.assertEqual(self.data(), {"version": 2, "items": {"MILK": {"name": "Milk", "lots": [
            {"qty": 5, "expires": "2026-12-01", "received": T},
            {"qty": 3, "expires": None, "received": "2026-10-12"}]}}})

    def test_name_defaults_and_rename(self):
        self.ok("add", "A", "1")
        self.ok("add", "A", "1", "--name", "Apple")
        self.ok("add", "A", "1")
        self.assertEqual(self.ok("list"), "A\tApple\t3\n")

    def test_remove_order(self):
        self.ok("add", "X", "2")                                   # no expiry
        self.ok("add", "X", "2", "--expires", "2026-11-05")
        self.ok("add", "X", "2", "--expires", "2026-11-01")
        self.ok("add", "X", "2", "--expires", "2026-11-05")
        self.ok("remove", "X", "3")
        lots = self.data()["items"]["X"]["lots"]
        self.assertEqual([(l["qty"], l["expires"]) for l in lots],
                         [(2, None), (1, "2026-11-05"), (2, "2026-11-05")])
        self.ok("remove", "X", "4")
        lots = self.data()["items"]["X"]["lots"]
        self.assertEqual([(l["qty"], l["expires"]) for l in lots], [(1, None)])
        self.ok("remove", "X", "1")
        self.assertEqual(self.data()["items"]["X"], {"name": "X", "lots": []})
        self.assertEqual(self.ok("list"), "X\tX\t0\n")

    def test_expired_stock_is_not_available(self):
        self.ok("add", "Y", "4", "--expires", "2026-10-10")
        self.ok("add", "Y", "1", "--expires", T)
        self.assertEqual(self.ok("list"), "Y\tY\t1\n")
        r = self.inv("remove", "Y", "2")
        self.assertEqual(r.returncode, 2)
        self.assertIn("error: insufficient stock for Y: have 1, need 2", r.stderr)
        before = self.data()
        self.assertEqual(len(before["items"]["Y"]["lots"]), 2)
        self.ok("remove", "Y", "1")
        self.assertEqual([l["qty"] for l in self.data()["items"]["Y"]["lots"]], [4])

    def test_insufficient_changes_nothing(self):
        self.ok("add", "Z", "2", "--expires", "2026-12-01")
        self.ok("add", "Z", "2")
        before = self.data()
        r = self.inv("remove", "Z", "5")
        self.assertEqual((r.returncode, r.stdout), (2, ""))
        self.assertIn("have 4, need 5", r.stderr)
        self.assertEqual(self.data(), before)
        r = self.inv("remove", "NOPE", "1")
        self.assertEqual(r.returncode, 2)
        self.assertIn("insufficient stock for NOPE: have 0, need 1", r.stderr)


class Reports(Base):
    def setUp(self):
        super().setUp()
        self.ok("add", "B", "1", "--expires", "2026-10-20")
        self.ok("add", "A", "2", "--expires", "2026-10-20")
        self.ok("add", "A", "3", "--expires", "2026-10-01")
        self.ok("add", "C", "4", "--expires", "2026-10-11")
        self.ok("add", "A", "5", "--expires", "2026-10-20")
        self.ok("add", "D", "6")
        self.ok("add", "E", "7", "--expires", "2027-01-01")

    def test_expiring(self):
        self.assertEqual(self.ok("expiring", "9"),
                         "2026-10-01\tA\t3 (expired)\n2026-10-11\tC\t4\n2026-10-20\tA\t2\n2026-10-20\tA\t5\n2026-10-20\tB\t1\n")
        self.assertEqual(self.ok("expiring", "0"), "2026-10-01\tA\t3 (expired)\n2026-10-11\tC\t4\n")
        self.assertEqual(self.ok("expiring", "0", today="2026-09-01"), "")

    def test_expiring_bad_days(self):
        for bad in ("-1", "x"):
            self.assertEqual(self.inv("expiring", bad).returncode, 1)

    def test_purge(self):
        self.assertEqual(self.ok("purge", today="2026-10-12"),
                         "purged A 2026-10-01 3\npurged C 2026-10-11 4\npurged 2 lots\n")
        items = self.data()["items"]
        self.assertEqual([l["qty"] for l in items["A"]["lots"]], [2, 5])
        self.assertEqual(items["C"]["lots"], [])
        self.assertEqual(self.ok("purge", today="2026-10-12"), "purged 0 lots\n")

    def test_list_counts_unexpired(self):
        self.assertEqual(self.ok("list"), "A\tA\t7\nB\tB\t1\nC\tC\t4\nD\tD\t6\nE\tE\t7\n")
        self.assertEqual(self.ok("list", today="2026-10-21"), "A\tA\t0\nB\tB\t0\nC\tC\t0\nD\tD\t6\nE\tE\t7\n")

    def test_reports_do_not_write(self):
        mtime = os.stat(self.db).st_mtime_ns
        with open(self.db, "rb") as f:
            before = f.read()
        self.ok("list")
        self.ok("expiring", "30")
        with open(self.db, "rb") as f:
            self.assertEqual(f.read(), before)
        self.assertEqual(os.stat(self.db).st_mtime_ns, mtime)


class Migration(Base):
    V1 = {"version": 1, "items": {"A-1": {"name": "Anvil", "qty": 3}, "B-2": {"name": "Bolt", "qty": 0}}}

    def test_reads_v1_without_writing(self):
        self.write(self.V1)
        self.assertEqual(self.ok("list"), "A-1\tAnvil\t3\nB-2\tBolt\t0\n")
        self.assertEqual(self.data(), self.V1)

    def test_change_writes_v2(self):
        self.write(self.V1)
        self.ok("remove", "A-1", "1")
        self.assertEqual(self.data(), {"version": 2, "items": {
            "A-1": {"name": "Anvil", "lots": [{"qty": 2, "expires": None, "received": "1970-01-01"}]},
            "B-2": {"name": "Bolt", "lots": []}}})

    def test_migrate_command(self):
        self.write(self.V1)
        self.assertEqual(self.ok("migrate"), "migrated 2 items\n")
        self.assertEqual(self.data()["version"], 2)
        self.assertEqual(self.data()["items"]["A-1"]["lots"], [{"qty": 3, "expires": None, "received": "1970-01-01"}])
        with open(self.db, "rb") as f:
            before = f.read()
        self.assertEqual(self.ok("migrate"), "already at version 2\n")
        with open(self.db, "rb") as f:
            self.assertEqual(f.read(), before)

    def test_migrate_missing_file(self):
        self.assertEqual(self.ok("migrate"), "migrated 0 items\n")
        self.assertEqual(self.data(), {"version": 2, "items": {}})

    def test_unsupported_version(self):
        self.write({"version": 7, "items": {}})
        for args in (("list",), ("migrate",), ("add", "A", "1")):
            r = self.inv(*args)
            self.assertEqual(r.returncode, 1, args)
            self.assertIn("error: unsupported version 7", r.stderr)


class Validation(Base):
    def test_bad_dates(self):
        r = self.inv("add", "A", "1", "--expires", "2026-13-01")
        self.assertEqual(r.returncode, 1)
        self.assertIn("error: bad date 2026-13-01", r.stderr)
        r = self.inv("list", today="yesterday")
        self.assertEqual(r.returncode, 1)
        self.assertIn("error: bad date yesterday", r.stderr)
        self.assertFalse(os.path.exists(self.db))

    def test_bad_quantity(self):
        for q in ("0", "-2", "x"):
            r = self.inv("add", "A", q)
            self.assertEqual(r.returncode, 1)
            self.assertIn("error: quantity must be a positive integer", r.stderr)

    def test_real_date_by_default(self):
        import datetime
        self.ok("add", "A", "1", today=None)
        self.assertEqual(self.data()["items"]["A"]["lots"][0]["received"], datetime.date.today().isoformat())


if __name__ == "__main__":
    unittest.main()
