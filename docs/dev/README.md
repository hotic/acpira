# Implementation notes

Notes for maintainers and coding agents working on Acpira itself, not user documentation. `AGENTS.md` at the repository root indexes them by area; read the matching file before changing that area.

An entry holds only what the source cannot tell: the wire behaviour of the agent CLIs, with the version it was observed on; invariants and cross-file contracts; the non-obvious why; traps; where an area lives. It describes the code as it is now and points at the source rather than restating it. Change history lives in git, not here. The writing rules are in `AGENTS.md`.

User-facing documentation is the [README](../../README.md); the public extension API is [`docs/context-api.md`](../context-api.md).
