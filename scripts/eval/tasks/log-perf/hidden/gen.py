"""Deterministic access-log generator for the hidden tests."""

import random

METHODS = ["GET", "GET", "GET", "POST", "PUT", "DELETE", "PATCH"]
STATUS = [200] * 30 + [201, 204, 301, 304, 400, 401, 403, 404, 404, 429, 500, 502, 503]


def lines(n, seed, users=60000, routes=300):
    rnd = random.Random(seed)
    bases = [f"/api/v{rnd.randint(1, 3)}/{rnd.choice(['items', 'orders', 'users', 'carts', 'search'])}{i}" for i in range(routes)]
    out = []
    for i in range(n):
        r = rnd.random()
        if r < 0.01:
            out.append(rnd.choice(["", "   ", "garbage", "2026-10-11T08:00:00Z GET /x 200", "2026-10-11T08:00:00Z FETCH /x 200 3 u1",
                                   "2026-10-11T08:00:00Z GET /x 2000 3 u1", "2026-10-11 08:00:00Z GET /x 200 3 u1"]))
            continue
        sec = rnd.randint(0, 86399) if rnd.random() < 0.7 else rnd.randint(36000, 36600)
        ts = f"2026-10-11T{sec // 3600:02d}:{sec // 60 % 60:02d}:{sec % 60:02d}Z"
        path = rnd.choice(bases)
        if rnd.random() < 0.6:
            path += f"/{rnd.randint(1, 99999)}"
        if rnd.random() < 0.2:
            path += f"/sub{rnd.randint(1, 3)}"
        if rnd.random() < 0.15:
            path += f"?page={rnd.randint(1, 9)}"
        ms = int(rnd.expovariate(1 / 80)) + (rnd.randint(500, 5000) if rnd.random() < 0.02 else 0)
        pad = " " if rnd.random() < 0.01 else ""
        out.append(f"{pad}{ts} {rnd.choice(METHODS)} {path} {rnd.choice(STATUS)} {ms} u{rnd.randint(1, users)}{pad}")
    return out
