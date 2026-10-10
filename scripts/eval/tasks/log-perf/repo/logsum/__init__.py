"""logsum: summarise an access log.

Each line is `TIMESTAMP METHOD PATH STATUS MS USER`, for example

    2026-10-11T08:15:02Z GET /api/items/42 200 35 u1203

Anything else is counted as malformed. `summarize(lines)` returns the report as one string.
"""

import math
import re


def _percentile(values, p):
    # Nearest-rank percentile
    s = sorted(values)
    k = max(0, math.ceil(p / 100 * len(s)) - 1)
    return s[k]


def _normalize(path):
    # Numeric path segments become :id, the query string is dropped
    path = path.split("?")[0]
    return "/".join(":id" if re.fullmatch(r"\d+", seg) else seg for seg in path.split("/"))


def summarize(lines):
    records = []
    malformed = 0
    users = []
    for line in lines:
        line = line.strip()
        if not line:
            continue
        m = re.compile(r"(\d{4}-\d\d-\d\dT\d\d:\d\d):\d\dZ (GET|POST|PUT|DELETE|PATCH) (\S+) (\d{3}) (\d+) (\S+)").fullmatch(line)
        if not m:
            malformed += 1
            continue
        minute, method, path, status, ms, user = m.groups()
        records.append((minute, method, _normalize(path), int(status), int(ms), user))
        if user not in users:
            users.append(user)

    out = []
    out.append(f"lines: {len(records) + malformed}")
    out.append(f"malformed: {malformed}")
    out.append(f"users: {len(users)}")
    if not records:
        return "\n".join(out) + "\n"

    classes = {}
    for r in records:
        c = f"{r[3] // 100}xx"
        classes[c] = classes.get(c, 0) + 1
    out.append("status: " + " ".join(f"{c}={classes[c]}" for c in sorted(classes)))

    lat = [r[4] for r in records]
    out.append(f"latency ms: p50={_percentile(lat, 50)} p90={_percentile(lat, 90)} p99={_percentile(lat, 99)} max={max(lat)}")

    # Top endpoints by count (ties by method then path), with their average and p90 latency
    endpoints = []
    for r in records:
        key = (r[1], r[2])
        if key not in endpoints:
            endpoints.append(key)
    stats = []
    for key in endpoints:
        ms = [r[4] for r in records if (r[1], r[2]) == key]
        errors = len([r for r in records if (r[1], r[2]) == key and r[3] >= 500])
        stats.append((key, len(ms), sum(ms) / len(ms), _percentile(ms, 90), errors))
    stats.sort(key=lambda s: (-s[1], s[0]))
    out.append("top endpoints:")
    for (method, path), n, avg, p90, errors in stats[:10]:
        out.append(f"  {method} {path} count={n} avg={avg:.1f} p90={p90} 5xx={errors}")

    # The busiest minutes (ties: earliest first)
    minutes = []
    for r in records:
        for pair in minutes:
            if pair[0] == r[0]:
                pair[1] += 1
                break
        else:
            minutes.append([r[0], 1])
    minutes.sort(key=lambda p: (-p[1], p[0]))
    out.append("busiest minutes: " + ", ".join(f"{m}={n}" for m, n in minutes[:3]))

    # Users with the most server errors (at least one), ties by user id
    errs = {}
    for u in users:
        n = sum(1 for r in records if r[5] == u and r[3] >= 500)
        if n:
            errs[u] = n
    worst = sorted(errs.items(), key=lambda kv: (-kv[1], kv[0]))[:5]
    out.append("most 5xx: " + (", ".join(f"{u}={n}" for u, n in worst) if worst else "none"))
    return "\n".join(out) + "\n"
