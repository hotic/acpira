import os
import subprocess
import sys
import tempfile
import unittest


class Cli(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.db = os.path.join(self.dir.name, "inv.json")

    def tearDown(self):
        self.dir.cleanup()

    def inv(self, *args):
        return subprocess.run([sys.executable, "-m", "inv", "--db", self.db, *args], capture_output=True, text=True)

    def test_add_remove_list(self):
        self.assertEqual(self.inv("add", "B-2", "5", "--name", "Bolt").returncode, 0)
        self.assertEqual(self.inv("add", "A-1", "3").returncode, 0)
        self.assertEqual(self.inv("remove", "B-2", "2").returncode, 0)
        self.assertEqual(self.inv("list").stdout, "A-1\tA-1\t3\nB-2\tBolt\t3\n")

    def test_insufficient(self):
        self.inv("add", "A-1", "1")
        r = self.inv("remove", "A-1", "2")
        self.assertEqual(r.returncode, 2)
        self.assertIn("insufficient stock for A-1: have 1, need 2", r.stderr)

    def test_bad_quantity(self):
        self.assertEqual(self.inv("add", "A-1", "zero").returncode, 1)


if __name__ == "__main__":
    unittest.main()
