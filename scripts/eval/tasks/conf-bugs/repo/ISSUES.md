# Open issues

1. `homepage = https://example.com/#about` reads back as `https://example.com/`. Also `color = #ff0000` comes back empty.
2. Multi-line values come back on one line: a `motd` written over three indented lines should keep its line breaks.
3. `debug = False` raises `ValueError: not a boolean`, while `debug = false` works.
4. A section cannot override a value it inherits from `[DEFAULT]`: with `port = 80` in DEFAULT and `port = 8080` in
   `[web]`, `get("web", "port")` returns `80`.
5. `price = $$5` reads back as `$$5` instead of `$5`.
6. A value that refers to itself (`a = ${a}`), or two values that refer to each other, crash with `RecursionError`
   instead of raising `InterpolationError`.
7. `${paths:logs}` where `[paths]` has `base = /var` and `logs = ${base}/logs` fails (or picks up the wrong `base`) when
   used from a section that has no `base` of its own, or one with a different `base`.
8. Error messages point at the line before the real one: a bad third line is reported as `line 2`.
