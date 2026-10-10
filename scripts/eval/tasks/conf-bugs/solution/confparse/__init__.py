"""confparse: an INI-style configuration reader.

Format (see README.md for the full description):

    ; a comment            # also a comment
    [DEFAULT]
    root = /srv
    [web]
    port = 8080
    docs = ${root}/docs     ; interpolation, inline comment
    motd = first line
       second line          (indented lines continue the previous value)
"""

import re

__all__ = ["Config", "ParseError", "InterpolationError", "parse", "load"]

DEFAULT = "DEFAULT"
_TRUE = {"1", "yes", "true", "on"}
_FALSE = {"0", "no", "false", "off"}
_REF = re.compile(r"\$(\$|\{([^}]*)\})")
_MAX_DEPTH = 10


class ParseError(Exception):
    def __init__(self, line, msg):
        super().__init__(f"line {line}: {msg}")
        self.line = line


class InterpolationError(Exception):
    pass


def _strip_inline_comment(value):
    # An inline comment starts at ';' or '#' preceded by whitespace
    m = re.search(r"\s[;#]", value)
    return value[: m.start()].rstrip() if m else value


def parse(text):
    sections = {DEFAULT: {}}
    order = []
    current = None
    last_key = None
    for no, raw in enumerate(text.splitlines(), start=1):
        line = raw.rstrip()
        stripped = line.strip()
        if not stripped or stripped[0] in "#;":
            # Blank lines and comments end a continued value
            last_key = None if not stripped else last_key
            continue
        if raw[0] in " \t":
            if current is None or last_key is None:
                raise ParseError(no, "unexpected indented line")
            sections[current][last_key] += "\n" + _strip_inline_comment(stripped)
            continue
        if stripped.startswith("["):
            if not stripped.endswith("]") or len(stripped) < 3:
                raise ParseError(no, f"bad section header {stripped!r}")
            current = stripped[1:-1].strip()
            if current not in sections:
                sections[current] = {}
                if current != DEFAULT:
                    order.append(current)
            last_key = None
            continue
        m = re.match(r"([^=:]+?)\s*[=:]\s*(.*)$", stripped)
        if not m:
            raise ParseError(no, f"expected 'key = value', got {stripped!r}")
        if current is None:
            raise ParseError(no, "key outside a section")
        key = m.group(1).strip().lower()
        sections[current][key] = _strip_inline_comment(m.group(2))
        last_key = key
    return Config(sections, order)


def load(path):
    with open(path, encoding="utf-8") as f:
        return parse(f.read())


class Config:
    def __init__(self, sections, order):
        self._sections = sections
        self._order = order

    def sections(self):
        """Section names in file order, DEFAULT excluded."""
        return list(self._order)

    def has_section(self, name):
        return name in self._sections and name != DEFAULT

    def _raw_items(self, section):
        if section not in self._sections:
            raise KeyError(section)
        # A section's own keys override the inherited DEFAULT ones
        merged = dict(self._sections[DEFAULT])
        merged.update(self._sections[section])
        return merged

    def keys(self, section):
        """Keys of a section, its own first in file order, then inherited DEFAULT keys."""
        own = list(self._sections[section].keys()) if section in self._sections else None
        if own is None:
            raise KeyError(section)
        return own + [k for k in self._sections[DEFAULT] if k not in own]

    def get_raw(self, section, key):
        items = self._raw_items(section)
        key = key.lower()
        if key not in items:
            raise KeyError(f"{section}.{key}")
        return items[key]

    def get(self, section, key, default=None):
        try:
            raw = self.get_raw(section, key)
        except KeyError:
            if default is not None:
                return default
            raise
        return self._interpolate(section, raw, 0)

    def _interpolate(self, section, value, depth):
        if depth > _MAX_DEPTH:
            raise InterpolationError(f"interpolation too deep in [{section}]")

        def sub(m):
            if m.group(1) == "$":
                return "$"
            ref = m.group(2)
            sec, _, key = ref.rpartition(":")
            sec = sec or section
            try:
                raw = self.get_raw(sec, key)
            except KeyError:
                raise InterpolationError(f"bad reference ${{{ref}}} in [{section}]") from None
            return self._interpolate(sec, raw, depth + 1)

        return _REF.sub(sub, value)

    def getint(self, section, key):
        return int(self.get(section, key))

    def getfloat(self, section, key):
        return float(self.get(section, key))

    def getbool(self, section, key):
        v = self.get(section, key).strip().lower()
        if v in _TRUE:
            return True
        if v in _FALSE:
            return False
        raise ValueError(f"not a boolean: {v!r}")

    def getlist(self, section, key):
        """Comma separated items, stripped, empty items dropped."""
        return [x.strip() for x in self.get(section, key).split(",") if x.strip()]
