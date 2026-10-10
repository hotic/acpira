"""logsum: summarise an access log.

Each line is `TIMESTAMP METHOD PATH STATUS MS USER`, for example

    2026-10-11T08:15:02Z GET /api/items/42 200 35 u1203

Anything else is counted as malformed. `summarize(lines)` returns the report as one string.
"""

import math
import re

_LINE = re.compile(r"(\d{4}-\d\d-\d\dT\d\d:\d\d):\d\dZ (GET|POST|PUT|DELETE|PATCH) (\S+) (\d{3}) (\d+) (\S+)")
_DIGITS = re.compile(r"\d+")


def _rank(sorted_values, p):
    # Nearest-rank percentile of an already sorted list
    return sorted_values[max(0, math.ceil(p / 100 * len(sorted_values)) - 1)]


def _percentile(values, p):
    return _rank(sorted(values), p)


_norm_cache = {}


def _normalize(path):
    # Numeric path segments become :id, the query string is dropped; paths repeat a lot, so cache them
    got = _norm_cache.get(path)
    if got is None:
        got = "/".join(":id" if _DIGITS.fullmatch(seg) else seg for seg in path.split("?")[0].split("/"))
        if len(_norm_cache) < 100_000:
            _norm_cache[path] = got
    return got


def summarize(lines):
    malformed = 0
    total = 0
    users = set()
    classes = {}
    lat = []
    endpoints = {}  # (method, path) -> [latencies, 5xx count]
    minutes = {}
    errs = {}
    match = _LINE.fullmatch
    for line in lines:
        line = line.strip()
        if not line:
            continue
        m = match(line)
        if not m:
            malformed += 1
            continue
        total += 1
        minute, method, path, status, ms, user = m.groups()
        status = int(status)
        ms = int(ms)
        users.add(user)
        c = status // 100
        classes[c] = classes.get(c, 0) + 1
        lat.append(ms)
        e = endpoints.get((method, _normalize(path)))
        if e is None:
            e = endpoints[(method, _normalize(path))] = [[], 0]
        e[0].append(ms)
        minutes[minute] = minutes.get(minute, 0) + 1
        if status >= 500:
            e[1] += 1
            errs[user] = errs.get(user, 0) + 1

    out = [f"lines: {total + malformed}", f"malformed: {malformed}", f"users: {len(users)}"]
    if not total:
        return "\n".join(out) + "\n"

    # Status classes sort as the strings "1xx".."5xx" did; a 3-digit status gives one digit
    out.append("status: " + " ".join(f"{c}xx={classes[c]}" for c in sorted(classes, key=lambda c: f"{c}xx")))

    lat.sort()
    out.append(f"latency ms: p50={_rank(lat, 50)} p90={_rank(lat, 90)} p99={_rank(lat, 99)} max={lat[-1]}")

    stats = sorted(((k, len(v[0])) for k, v in endpoints.items()), key=lambda s: (-s[1], s[0]))
    out.append("top endpoints:")
    for (method, path), n in stats[:10]:
        ms, errors = endpoints[(method, path)]
        out.append(f"  {method} {path} count={n} avg={sum(ms) / n:.1f} p90={_percentile(ms, 90)} 5xx={errors}")

    busiest = sorted(minutes.items(), key=lambda p: (-p[1], p[0]))[:3]
    out.append("busiest minutes: " + ", ".join(f"{m}={n}" for m, n in busiest))

    worst = sorted(errs.items(), key=lambda kv: (-kv[1], kv[0]))[:5]
    out.append("most 5xx: " + (", ".join(f"{u}={n}" for u, n in worst) if worst else "none"))
    return "\n".join(out) + "\n"
