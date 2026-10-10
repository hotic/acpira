"""calc: a small scripting language. The language is described in SPEC.md."""

__all__ = ["CalcError", "run"]


class CalcError(Exception):
    """Raised for every syntax or runtime error, with the message `line N: <message>`."""


def run(source: str) -> list[str]:
    """Execute a program and return the lines it printed."""
    raise NotImplementedError
