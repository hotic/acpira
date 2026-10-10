import unittest

import confparse


class Basic(unittest.TestCase):
    def test_sections_and_keys(self):
        c = confparse.parse("[a]\nX = 1\ny: two\n[b]\nz=3\n")
        self.assertEqual(c.sections(), ["a", "b"])
        self.assertEqual(c.get("a", "x"), "1")
        self.assertEqual(c.get("a", "Y"), "two")
        self.assertEqual(c.getint("b", "z"), 3)

    def test_interpolation(self):
        c = confparse.parse("[a]\nroot = /srv\ndocs = ${root}/docs\n[b]\nd = ${a:root}/x\n")
        self.assertEqual(c.get("a", "docs"), "/srv/docs")
        self.assertEqual(c.get("b", "d"), "/srv/x")

    def test_list(self):
        c = confparse.parse("[a]\nhosts = x, y,, z \n")
        self.assertEqual(c.getlist("a", "hosts"), ["x", "y", "z"])


if __name__ == "__main__":
    unittest.main()
