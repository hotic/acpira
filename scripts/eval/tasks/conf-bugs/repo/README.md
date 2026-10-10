# confparse

An INI-style configuration reader. `confparse.parse(text)` and `confparse.load(path)` return a `Config`.

## Format

- `[name]` starts a section. Section names are case-sensitive; repeating a section adds to it. `[DEFAULT]` holds values
  every section inherits; a section's own value wins over the inherited one.
- `key = value` or `key: value`. Keys are case-insensitive (stored lower-case); surrounding whitespace is dropped. A key
  repeated in one section keeps the last value.
- Whole-line comments start with `#` or `;`. An inline comment starts at `#` or `;` preceded by whitespace; any other `#`
  or `;` is part of the value.
- An indented line continues the previous value: it is appended after a newline, with its own indentation removed.
  A blank line ends the value.
- `${key}` refers to another key of the same section (DEFAULT included), `${section:key}` to a key of another section;
  a referenced value is interpolated in its own section. `$$` is a literal `$`. A missing reference raises
  `InterpolationError`, and so do references nested more than 10 levels deep (cycles included).
- Syntax errors raise `ParseError` with the message `line N: ...`, lines counted from 1.

## API

- `Config.sections()`: section names in file order, DEFAULT excluded. `has_section(name)`.
- `keys(section)`: own keys in file order, then inherited DEFAULT keys.
- `get(section, key, default=None)`: interpolated value; `get_raw` returns it as written.
- `getint`, `getfloat`, `getbool` (`1 yes true on` / `0 no false off`, any case), `getlist` (comma separated, items
  stripped, empty items dropped).

Tests: `python3 -m unittest discover -s tests`.
