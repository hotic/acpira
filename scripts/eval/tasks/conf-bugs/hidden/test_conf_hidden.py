import os
import tempfile
import unittest

from confparse import InterpolationError, ParseError, load, parse


class Issues(unittest.TestCase):
    def test_1_hash_inside_values(self):
        c = parse("[a]\nhomepage = https://example.com/#about\ncolor = #ff0000\nx = a;b ; comment\ny = q # c\n")
        self.assertEqual(c.get("a", "homepage"), "https://example.com/#about")
        self.assertEqual(c.get("a", "color"), "#ff0000")
        self.assertEqual(c.get("a", "x"), "a;b")
        self.assertEqual(c.get("a", "y"), "q")

    def test_2_multiline(self):
        c = parse("[a]\nmotd = first\n   second line\n\tthird ; note\nnext = 1\n")
        self.assertEqual(c.get("a", "motd"), "first\nsecond line\nthird")
        self.assertEqual(c.get("a", "next"), "1")

    def test_2_blank_line_ends_value(self):
        with self.assertRaises(ParseError) as e:
            parse("[a]\nk = v\n\n  orphan\n")
        self.assertTrue(str(e.exception).startswith("line 4:"), str(e.exception))

    def test_3_bool_any_case(self):
        c = parse("[a]\na = False\nb = TRUE\nc = On\nd = no\ne = 1\nf = maybe\n")
        self.assertEqual([c.getbool("a", k) for k in "abcde"], [False, True, True, False, True])
        with self.assertRaises(ValueError):
            c.getbool("a", "f")

    def test_4_own_value_wins(self):
        c = parse("[DEFAULT]\nport = 80\nhost = localhost\n[web]\nport = 8080\n[db]\n")
        self.assertEqual(c.get("web", "port"), "8080")
        self.assertEqual(c.get("db", "port"), "80")
        self.assertEqual(c.get("web", "host"), "localhost")

    def test_4_default_value_sees_section_override(self):
        c = parse("[DEFAULT]\nhost = localhost\nurl = http://${host}/\n[web]\nhost = example.org\n")
        self.assertEqual(c.get("web", "url"), "http://example.org/")

    def test_5_dollar_escape(self):
        c = parse("[a]\nprice = $$5\nmix = $${x} and ${y}\ny = Y\n")
        self.assertEqual(c.get("a", "price"), "$5")
        self.assertEqual(c.get("a", "mix"), "${x} and Y")
        self.assertEqual(c.get_raw("a", "price"), "$$5")

    def test_6_cycles(self):
        c = parse("[a]\nself = ${self}\np = ${q}\nq = ${p}\n")
        for k in ("self", "p"):
            with self.assertRaises(InterpolationError):
                c.get("a", k)

    def test_6_deep_but_finite_chain_works(self):
        lines = ["[a]", "k0 = end"] + [f"k{i} = ${{k{i - 1}}}" for i in range(1, 9)]
        c = parse("\n".join(lines))
        self.assertEqual(c.get("a", "k8"), "end")

    def test_7_reference_resolved_in_its_own_section(self):
        c = parse("[paths]\nbase = /var\nlogs = ${base}/logs\n[app]\nlog = ${paths:logs}/app.log\n[other]\nbase = /tmp\nlog = ${paths:logs}\n")
        self.assertEqual(c.get("app", "log"), "/var/logs/app.log")
        self.assertEqual(c.get("other", "log"), "/var/logs")

    def test_8_line_numbers(self):
        cases = {
            "[a]\nk = v\n=oops\n": 3,
            "# c\n[a\n": 2,
            "k = v\n": 1,
            "[a]\n\n\n   indented\n": 4,
        }
        for text, line in cases.items():
            with self.assertRaises(ParseError) as e:
                parse(text)
            self.assertTrue(str(e.exception).startswith(f"line {line}:"), (text, str(e.exception)))
            self.assertEqual(e.exception.line, line)


class Regressions(unittest.TestCase):
    def test_sections_order_and_merge(self):
        c = parse("[b]\nx = 1\n[DEFAULT]\nd = 0\n[a]\ny = 2\n[b]\nz = 3\nx = 4\n")
        self.assertEqual(c.sections(), ["b", "a"])
        self.assertEqual(c.keys("b"), ["x", "z", "d"])
        self.assertEqual(c.get("b", "x"), "4")
        self.assertTrue(c.has_section("a"))
        self.assertFalse(c.has_section("DEFAULT"))
        self.assertFalse(c.has_section("B"))

    def test_keys_case_insensitive(self):
        c = parse("[S]\nMixed_Key: Value Here\n")
        self.assertEqual(c.get("S", "mixed_key"), "Value Here")
        self.assertEqual(c.get("S", "MIXED_KEY"), "Value Here")

    def test_missing(self):
        c = parse("[a]\nx = ${nope}\n")
        with self.assertRaises(InterpolationError):
            c.get("a", "x")
        with self.assertRaises(KeyError):
            c.get("a", "y")
        with self.assertRaises(KeyError):
            c.get("zz", "x")
        self.assertEqual(c.get("a", "y", "fallback"), "fallback")

    def test_numbers_and_lists(self):
        c = parse("[n]\ni = 42\nf = 2.5\nl = a, b ,c\nempty =\n")
        self.assertEqual(c.getint("n", "i"), 42)
        self.assertEqual(c.getfloat("n", "f"), 2.5)
        self.assertEqual(c.getlist("n", "l"), ["a", "b", "c"])
        self.assertEqual(c.getlist("n", "empty"), [])
        self.assertEqual(c.get("n", "empty"), "")

    def test_comments(self):
        c = parse("; top\n# top\n[a]\n  ; indented comment\nk = v\n")
        self.assertEqual(c.get("a", "k"), "v")

    def test_value_with_separator(self):
        c = parse("[a]\nurl = http://h:8080/x=1\n")
        self.assertEqual(c.get("a", "url"), "http://h:8080/x=1")

    def test_load(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "c.ini")
            with open(p, "w", encoding="utf-8") as f:
                f.write("[a]\nname = café\n")
            self.assertEqual(load(p).get("a", "name"), "café")


if __name__ == "__main__":
    unittest.main()
