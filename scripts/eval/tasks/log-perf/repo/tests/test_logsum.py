import unittest

from logsum import summarize

SAMPLE = """\
2026-10-11T08:15:02Z GET /api/items/42 200 35 u1
2026-10-11T08:15:09Z GET /api/items/7?full=1 200 15 u2
2026-10-11T08:16:00Z POST /api/orders 503 900 u1
garbage line
2026-10-11T08:16:30Z GET /health 200 1 u3
"""


class Sample(unittest.TestCase):
    def test_report(self):
        self.assertEqual(summarize(SAMPLE.splitlines()), """\
lines: 5
malformed: 1
users: 3
status: 2xx=3 5xx=1
latency ms: p50=15 p90=900 p99=900 max=900
top endpoints:
  GET /api/items/:id count=2 avg=25.0 p90=35 5xx=0
  GET /health count=1 avg=1.0 p90=1 5xx=0
  POST /api/orders count=1 avg=900.0 p90=900 5xx=1
busiest minutes: 2026-10-11T08:15=2, 2026-10-11T08:16=2
most 5xx: u1=1
""")

    def test_empty(self):
        self.assertEqual(summarize([]), "lines: 0\nmalformed: 0\nusers: 0\n")


if __name__ == "__main__":
    unittest.main()
