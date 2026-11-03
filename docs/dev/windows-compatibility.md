# Windows compatibility

Windows boundaries the code works around, their checks, and what lacks native acceptance. Platform code: `rust/crates/acpira-host/src/platform/`.

## Platform boundaries

- Executable lookup skips the same-named npm POSIX shim; batch files always go through `platform::command::spawn_spec`.
- Terminal and install scripts are built only in Rust (`platform/terminal.rs`); native argv goes via `ProcessStartInfo` because PowerShell 5.1 drops empty arguments and mangles double quotes.
- Devin 3000.11.3's install script overwrites the running EXE in place, so the old entry is kept and restored on failure. Devin uses the Roaming AppData Known Folder, ignoring XDG / APPDATA overrides (`accounts::devin`).
- Shared config links: never `cmd /c mklink` (it writes to sidecar stdout). File hard links stay on one volume and stop following the source once it is atomically replaced.
- File lock liveness treats access denied as alive (`store::file_lock`).
- File URLs are handled only in `platform::file_url`; encoded separators are rejected.
- `canonicalize` yields `\\?\` paths and expands 8.3 names that Node keeps, breaking Pi trust and Node module resolution: `platform::paths::canonical_for_cli` converts for external CLIs and Node; real file access keeps the original path.
- Agents run in a kill-on-close Job Object at below-normal priority under a CPU cap (`agentCpuCap`), since parallel runs under Defender starve the desktop in kernel time.
- Session lock takeover only targets a verified direct child of another sidecar (`lock_holder.rs`).
- ChatGPT bridge commands drain output after exit, since a helper can hold the pipe open.
- Remote / WSL: the backend OS decides command syntax; `agents.json` keeps local Windows and remote Linux config apart.
- Release sidecars: MSVC target, static CRT (`scripts/release/sidecar-targets.mjs`).
- Remote access: the logon task runs the GUI-subsystem engine (`serve --service --log`, the exe's own output redirected) so no console opens; owner-only control pipe and DACLs (`host-architecture.md`, Remote access gateway). Unverified on Windows: `/Create /F` over a running instance, `SetStdHandle` redirection.

## Regression checks

- `scripts/ci/test-windows.ps1` is the single native entry (push / PR and release CI). Compiling or a GNU cross-build does not prove the MSVC release build.
- `login_service_round_trip` (`--ignored`) registers a real logon task; the gateway end-to-end test needs `pnpm install`.
- Rust engine tests locate the repo via `canonical_for_cli` and pass the fake agent loader as a file URL; `\\?\` and `S:\…` paths otherwise fail Node.

## Open gaps

- CMD `%*` forwarding shims truncate an argument with embedded quotes followed by `& % ! ^`. Global double escaping would break batch files reading `%~1`; a fix must tell forwarding shims apart. Built-in login arguments do not hit it.
- Some native Rust engine tests and POSIX-only `pnpm test` cases (file modes, `fileLock`, `acpira serve` socket, bridge CLI) fail on Windows.
- Unverified: real OAuth per vendor, terminal TTY, IDE buttons and browser callbacks, native arm64, each vendor's stdio MCP launch, shared drives, redirected Known Folders, antivirus / enterprise policy, PowerShell or WSL interop disabled.
- Unverified for remote access: firewall prompt, `DnsServiceRegister` seen from an iPhone, Modern Standby.
- Desktop Commander detection reports `unknown`; an executable's presence is not shown as connected.
- Claude may use a user-level `ANTHROPIC_API_KEY`, so "not signed in" does not mean the CLI lacks credentials (`agent-quirks-claude.md`).
