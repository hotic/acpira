"""semverlite: a Python port of js/semver.js with identical behaviour.

Regexes use re.ASCII because JavaScript's \\d and \\s-free patterns only match ASCII digits, and fullmatch stands in for
JavaScript's ^...$ (Python's $ also matches before a trailing newline).
"""

import re

__all__ = ["parse", "format", "compare", "satisfies", "max_satisfying", "inc"]

_NUM = r"0|[1-9]\d*"
_ID = rf"(?:{_NUM}|\d*[a-zA-Z-][a-zA-Z0-9-]*)"
_FULL = re.compile(rf"v?({_NUM})\.({_NUM})\.({_NUM})(?:-({_ID}(?:\.{_ID})*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?", re.ASCII)
_PARTIAL = re.compile(rf"v?({_NUM}|[xX*])(?:\.({_NUM}|[xX*])(?:\.({_NUM}|[xX*])(?:-({_ID}(?:\.{_ID})*))?)?)?", re.ASCII)
_OPS = r"(<=|>=|<|>|=|\^|~)"
# JavaScript's \s: the Unicode white space plus the BOM, which Python's str.isspace() lacks
_JS_WS = "\t\n\v\f\r                  　﻿"
_WS = f"[{_JS_WS}]"


def _trim(s):
    return s.strip(_JS_WS)


def _pre_id(i):
    return int(i) if re.fullmatch(r"\d+", i, re.ASCII) else i


def parse(text):
    if not isinstance(text, str):
        return None
    m = _FULL.fullmatch(_trim(text))
    if not m:
        return None
    return {
        "major": int(m.group(1)),
        "minor": int(m.group(2)),
        "patch": int(m.group(3)),
        "prerelease": [_pre_id(i) for i in m.group(4).split(".")] if m.group(4) else [],
        "build": m.group(5).split(".") if m.group(5) else [],
    }


def format(v):
    s = f"{v['major']}.{v['minor']}.{v['patch']}"
    if v["prerelease"]:
        s += "-" + ".".join(str(x) for x in v["prerelease"])
    if v["build"]:
        s += "+" + ".".join(v["build"])
    return s


def _must(v):
    p = parse(v) if isinstance(v, str) else v
    if not p:
        raise ValueError(f"invalid version: {v}")
    return p


def _cmp_num(a, b):
    return -1 if a < b else 1 if a > b else 0


def _is_num(x):
    return isinstance(x, int) and not isinstance(x, bool)


def _compare_pre(a, b):
    # A version without prerelease ranks above one with it
    if not a and not b:
        return 0
    if not a:
        return 1
    if not b:
        return -1
    i = 0
    while True:
        if i >= len(a) and i >= len(b):
            return 0
        if i >= len(a):
            return -1
        if i >= len(b):
            return 1
        x, y = a[i], b[i]
        i += 1
        if x == y and type(x) is type(y):
            continue
        xn, yn = _is_num(x), _is_num(y)
        if xn and yn:
            return _cmp_num(x, y)
        if xn:
            return -1
        if yn:
            return 1
        # JavaScript compares UTF-16 code units; the identifiers are ASCII, so code points agree
        return -1 if x < y else 1


def compare(a, b):
    x, y = _must(a), _must(b)
    return (_cmp_num(x["major"], y["major"]) or _cmp_num(x["minor"], y["minor"])
            or _cmp_num(x["patch"], y["patch"]) or _compare_pre(x["prerelease"], y["prerelease"]))


def _is_x(p):
    return p is None or p in ("x", "X", "*")


def _ver(major, minor, patch, prerelease=None):
    return {"major": major, "minor": minor, "patch": patch, "prerelease": prerelease or [], "build": []}


def _partial(text):
    m = _PARTIAL.fullmatch(text)
    if not m:
        raise ValueError(f"invalid range: {text}")
    a, b, c, pre = m.groups()
    xs = [_is_x(p) for p in (a, b, c)]
    # Once a part is a wildcard every later part is too (1.x.3 is 1.x)
    n = xs.index(True) if True in xs else 3
    return {
        "parts": n,
        "major": int(a) if n > 0 else 0,
        "minor": int(b) if n > 1 else 0,
        "patch": int(c) if n > 2 else 0,
        "prerelease": parse(f"0.0.0-{pre}")["prerelease"] if n > 2 and pre else [],
        "build": [],
    }


def _above(p):
    # The upper bound right above a partial version: 1 -> 2.0.0, 1.2 -> 1.3.0
    return _ver(p["major"] + 1, 0, 0) if p["parts"] == 1 else _ver(p["major"], p["minor"] + 1, 0)


def _expand(op, text):
    p = _partial(text)
    lo = _ver(p["major"], p["minor"], p["patch"], p["prerelease"])
    parts = p["parts"]
    if op == "^":
        if parts == 0:
            return [(">=", _ver(0, 0, 0))]
        if p["major"] > 0 or parts == 1:
            hi = _ver(p["major"] + 1, 0, 0)
        elif p["minor"] > 0 or parts == 2:
            hi = _ver(0, p["minor"] + 1, 0)
        else:
            hi = _ver(0, 0, p["patch"] + 1)
        return [(">=", lo), ("<", hi)]
    if op == "~":
        if parts == 0:
            return [(">=", _ver(0, 0, 0))]
        return [(">=", lo), ("<", _ver(p["major"] + 1, 0, 0) if parts == 1 else _ver(p["major"], p["minor"] + 1, 0))]
    if parts == 3:
        return [(op or "=", lo)]
    if parts == 0:
        return [("<", _ver(0, 0, 0))] if op in ("<", ">") else [(">=", _ver(0, 0, 0))]
    # A partial version with an operator
    if op == ">":
        return [(">=", _above(p))]
    if op == ">=":
        return [(">=", lo)]
    if op == "<":
        return [("<", lo)]
    if op == "<=":
        return [("<", _above(p))]
    return [(">=", lo), ("<", _above(p))]


def _parse_set(text):
    t = re.sub(_OPS + _WS + "+", r"\1", _trim(text))
    hy = re.fullmatch(rf"([^{_JS_WS}]+){_WS}+-{_WS}+([^{_JS_WS}]+)", t)
    if hy:
        lo, hi = _partial(hy.group(1)), _partial(hy.group(2))
        out = [(">=", _ver(lo["major"], lo["minor"], lo["patch"], lo["prerelease"]))]
        if hi["parts"] == 3:
            out.append(("<=", _ver(hi["major"], hi["minor"], hi["patch"], hi["prerelease"])))
        elif hi["parts"] > 0:
            out.append(("<", _above(hi)))
        return out
    if t == "":
        return [(">=", _ver(0, 0, 0))]
    out = []
    for part in re.split(_WS + "+", t):
        m = re.fullmatch(_OPS + "?(.*)", part, re.DOTALL)
        out.extend(_expand(m.group(1) or "", m.group(2)))
    return out


def _parse_range(rng):
    if not isinstance(rng, str):
        raise ValueError("invalid range")
    return [_parse_set(s) for s in rng.split("||")]


def _test(c, v):
    op, cv = c
    r = compare(v, cv)
    return {"=": r == 0, "<": r < 0, "<=": r <= 0, ">": r > 0, ">=": r >= 0}.get(op, False)


def _set_allows(s, v):
    if not all(_test(c, v) for c in s):
        return False
    if not v["prerelease"]:
        return True
    # A prerelease only matches when the set names a prerelease of the same major.minor.patch
    return any(cv["prerelease"] and (cv["major"], cv["minor"], cv["patch"]) == (v["major"], v["minor"], v["patch"])
               for _, cv in s)


def satisfies(version, rng):
    sets = _parse_range(rng)
    v = parse(version)
    if not v:
        return False
    return any(_set_allows(s, v) for s in sets)


def max_satisfying(versions, rng):
    sets = _parse_range(rng)
    best = None
    for text in versions:
        v = parse(text)
        if not v or not any(_set_allows(s, v) for s in sets):
            continue
        if best is None or compare(v, best[1]) > 0:
            best = (text, v)
    return best[0] if best else None


def inc(version, release):
    v = _must(version)
    pre = v["prerelease"]
    if release == "major":
        # 2.0.0-rc.1 -> 2.0.0: a prerelease of a major release is finished by it
        return format(_ver(v["major"], 0, 0) if v["minor"] == 0 and v["patch"] == 0 and pre else _ver(v["major"] + 1, 0, 0))
    if release == "minor":
        return format(_ver(v["major"], v["minor"], 0) if v["patch"] == 0 and pre else _ver(v["major"], v["minor"] + 1, 0))
    if release == "patch":
        return format(_ver(v["major"], v["minor"], v["patch"]) if pre else _ver(v["major"], v["minor"], v["patch"] + 1))
    if release == "prerelease":
        if not pre:
            return format(_ver(v["major"], v["minor"], v["patch"] + 1, [0]))
        nxt = list(pre)
        i = len(nxt) - 1
        while i >= 0 and not _is_num(nxt[i]):
            i -= 1
        if i < 0:
            nxt.append(0)
        else:
            nxt[i] += 1
        return format(_ver(v["major"], v["minor"], v["patch"], nxt))
    raise ValueError(f"invalid release type: {release}")
