import os
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, os.path.dirname(__file__))

import fast_ref  # noqa: E402
import gen  # noqa: E402
import slow_ref  # noqa: E402
from logsum import summarize  # noqa: E402

BUDGET_S = 6.0


class SameOutput(unittest.TestCase):
    def test_random_small_logs(self):
        for seed in range(12):
            lines = gen.lines(1500, seed, users=40 if seed % 2 else 3000, routes=5 if seed % 3 == 0 else 60)
            self.assertEqual(summarize(lines), slow_ref.summarize(lines), f"seed {seed}")

    def test_edge_cases(self):
        cases = [
            [],
            ["", "  "],
            ["garbage", "more garbage"],
            ["2026-10-11T08:00:00Z GET /a/1/b/22?x=3 404 7 u1"],
            ["2026-10-11T08:00:00Z GET /a 200 5 u1", "2026-10-11T08:01:00Z GET /a 200 5 u2",
             "2026-10-11T08:01:30Z POST /a 200 5 u2", "2026-10-11T08:00:59Z POST /a 200 5 u1"],
            ["2026-10-11T08:00:00Z PATCH /x/007 099 0 z", "2026-10-11T08:00:00Z PUT /x/7 599 12 a",
             "2026-10-11T08:00:00Z PUT /x/7 500 13 a", "2026-10-11T08:00:00Z DELETE // 500 1 b"],
            [f"2026-10-11T08:{i % 60:02d}:00Z GET /p{i % 13} {200 + (i % 4) * 100} {i * 7 % 101} u{i % 9}" for i in range(400)],
        ]
        for lines in cases:
            self.assertEqual(summarize(lines), slow_ref.summarize(lines), lines[:3])

    def test_accepts_any_iterable(self):
        lines = gen.lines(300, 99)
        self.assertEqual(summarize(iter(lines)), slow_ref.summarize(lines))


class Speed(unittest.TestCase):
    def test_300k_lines(self):
        lines = gen.lines(300_000, 7)
        started = time.perf_counter()
        got = summarize(lines)
        took = time.perf_counter() - started
        self.assertEqual(got, fast_ref.summarize(lines))
        self.assertLess(took, BUDGET_S, f"took {took:.1f}s")

    def test_cli(self):
        lines = gen.lines(2000, 3)
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "access.log")
            with open(p, "w", encoding="utf-8") as f:
                f.write("\n".join(lines) + "\n")
            r = subprocess.run([sys.executable, "-m", "logsum", p], capture_output=True, text=True, timeout=60)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertEqual(r.stdout, slow_ref.summarize(lines))


if __name__ == "__main__":
    unittest.main()
