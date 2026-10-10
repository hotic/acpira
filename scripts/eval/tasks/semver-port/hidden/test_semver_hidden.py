"""Differential tests: semverlite against the original js/semver.js (a pristine copy) run under node."""

import itertools
import json
import os
import random
import subprocess
import unittest

import semverlite

HERE = os.path.dirname(os.path.abspath(__file__))
PY = {"parse": semverlite.parse, "format": semverlite.format, "compare": semverlite.compare,
      "satisfies": semverlite.satisfies, "maxSatisfying": semverlite.max_satisfying, "inc": semverlite.inc}

VERSIONS = [
    "1.2.3", "v1.2.3", " 1.2.3 ", "\t1.2.3\n", "1.2", "1", "01.2.3", "1.02.3", "1.2.03", "0.0.0", "10.20.30",
    "1.2.3-alpha", "1.2.3-alpha.1", "1.2.3-alpha.beta", "1.2.3-beta.2", "1.2.3-beta.11", "1.2.3-rc.1", "1.2.3-0",
    "1.2.3-0.3.7", "1.2.3-x.7.z.92", "1.2.3-01", "1.2.3-0a", "1.2.3-a-b", "1.2.3--", "1.2.3+build", "1.2.3+b.1.c",
    "1.2.3-rc.1+b", "1.2.3+", "1.2.3-", "1.2.3-a..b", "V1.2.3", "vv1.2.3", "1.2.3.4", "", "x.y.z", "1.2.x",
    "1.\u0662.3", "1.2.3\n", "1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta", "1.0.0-beta.2",
    "1.0.0-beta.11", "1.0.0-rc.1", "1.0.0", "2.0.0", "2.0.0-rc.1", "0.2.3", "0.2.9", "0.3.0", "0.0.3", "0.0.4",
    "1.3.0-beta", "1.3.0-beta.2", "1.9.9", "2.3.4", "2.3.9", "2.4.0", "3.0.0-0", "1.2.4-alpha", "123.456.789",
    "\u00a01.2.3", "1.2.3-B", "1.2.3-b", "1.2.3-10", "1.2.3-9",
    # JavaScript's \d is ASCII-only and its trim() takes the BOM, str.strip() does not
    "1.2\u0663.3", "1.2.3-\u0663", "1.2.3-a.\u0663", "\ufeff1.2.3", "1.2.3\u2028",
]

RANGES = [
    "", "*", "x", "1", "1.x", "1.2", "1.2.x", "1.x.3", "^1.2.3", "^0.2.3", "^0.0.3", "^1.2.x", "^0.x", "^0.0.x", "^0.0",
    "^1.2.3-beta.2", "^ 1.2.3", "~1.2.3", "~1.2", "~1", "~0.2.3", "~1.2.3-beta.1", "~ 1.2", ">1.2.3", ">=1.2.3",
    "<1.2.3", "<=1.2.3", "=1.2.3", "1.2.3", ">1.2", ">=1.2", "<1.2", "<=1.2", ">1", "<=1", ">*", "<*", ">=*",
    ">= 1.2.3 < 2", ">=1.3.0-beta.1 <2", "1.2.3 - 2.3.4", "1.2.3 - 2.3", "1.2 - 2", "1.2.3-rc.1 - 1.2.3",
    "1.2.3 - *", "~1.2 || ~1.4", "^1 || ^2.0.0-rc.1", "1.2.3 || ", " || ", "<1.0.0 || >=2.0.0", ">=1.0.0-beta",
    "^1.2.3 ^1.5.0", "1.2.3 - 2.3.4 || 3", ">=v1.2.3", "^v1.2.3", "!1.2.3", "1.2.3.4", ">=", "^", "a.b.c", "1.2.3 -",
    "1.2.3 2.3.4", "=1.2", "=*", "~0", "^0", ">=1.2.3-alpha <1.2.3", "1 - 1", "x - 2", "~1.2.3-beta.1 || 1.0.0-alpha",
]


def oracle(calls):
    r = subprocess.run(["node", os.path.join(HERE, "oracle.js"), os.path.join(HERE, "semver_orig.js")],
                       input=json.dumps(calls), capture_output=True, text=True, check=True)
    return json.loads(r.stdout)


def run_py(fn, args):
    try:
        return {"ok": PY[fn](*args)}
    except ValueError:
        return {"error": True}


class Differential(unittest.TestCase):
    def check(self, calls):
        want = oracle(calls)
        bad = []
        for call, w in zip(calls, want):
            got = run_py(*call)
            if json.dumps(got, sort_keys=True) != json.dumps(w, sort_keys=True):
                bad.append(f"{call[0]}{tuple(call[1])}: python {got} js {w}")
        if bad:
            self.fail(f"{len(bad)} of {len(calls)} differ:\n" + "\n".join(bad[:25]))

    def test_parse_and_format(self):
        calls = [["parse", [v]] for v in VERSIONS]
        calls += [["format", [p]] for p in (semverlite.parse(v) for v in VERSIONS) if p]
        self.check(calls)

    def test_compare(self):
        self.check([["compare", [a, b]] for a, b in itertools.product(VERSIONS[::2], VERSIONS[1::3])])

    def test_satisfies(self):
        self.check([["satisfies", [v, r]] for r in RANGES for v in VERSIONS])

    def test_max_satisfying(self):
        rnd = random.Random(5)
        calls = []
        for r in RANGES:
            for _ in range(4):
                calls.append(["maxSatisfying", [rnd.sample(VERSIONS, 12), r]])
        self.check(calls)

    def test_inc(self):
        kinds = ["major", "minor", "patch", "prerelease", "premajor", ""]
        extra = ["1.2.3-alpha", "1.2.3-alpha.1.beta", "1.2.0-rc", "1.0.0-x.1", "3.0.0-0", "0.1.0-a.b.c"]
        self.check([["inc", [v, k]] for v in VERSIONS + extra for k in kinds])

    def test_random_ranges(self):
        rnd = random.Random(11)
        ops = ["", "", "^", "~", ">", ">=", "<", "<=", "="]
        parts = ["0", "1", "2", "x", "*", "10"]

        def rv():
            n = rnd.randint(1, 3)
            s = ".".join(rnd.choice(parts) for _ in range(n))
            if n == 3 and rnd.random() < 0.3:
                s += "-" + rnd.choice(["alpha", "beta.1", "0", "rc.2"])
            return s

        calls = []
        for _ in range(300):
            sets = []
            for _ in range(rnd.randint(1, 2)):
                if rnd.random() < 0.2:
                    sets.append(f"{rv()} - {rv()}")
                else:
                    sets.append(" ".join(rnd.choice(ops) + rv() for _ in range(rnd.randint(1, 2))))
            r = " || ".join(sets)
            for v in rnd.sample(VERSIONS, 6):
                calls.append(["satisfies", [v, r]])
        self.check(calls)


if __name__ == "__main__":
    unittest.main()
