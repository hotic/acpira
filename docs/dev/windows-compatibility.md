# Windows compatibility

Windows platform boundaries the code works around, how they are regression-checked, and what still lacks native acceptance. Platform code lives in `rust/crates/acpira-host/src/platform/`.

## Platform boundaries

- Executable lookup skips the same-named npm POSIX shim for EXE / COM / CMD / BAT; batch files always go through `platform::command::spawn_spec`.
- Terminal and install scripts are built only in Rust (`platform/terminal.rs`, UTF-16LE encoded commands); TypeScript only delivers launch arguments. Native argv goes via `ProcessStartInfo` because PowerShell 5.1 drops empty arguments and mangles double quotes.
- Devin 3000.11.3's install script overwrites the running EXE in place: the old entry is kept and restored on failure. Devin on Windows uses the Roaming AppData Known Folder and ignores XDG / APPDATA overrides (`accounts::devin`).
- Shared config links: directories use symlinks or junctions, files fall back to hard links. Never `cmd /c mklink`: it writes to sidecar stdout. Hard links stay on one volume and stop following the source once it is atomically replaced.
- File lock liveness checks a native process handle and treats access denied as alive (`store::file_lock`); atomic writes keep std tmp + rename.
- File URLs (drive letters, UNC, `\\?\`, CJK) are handled only in `platform::file_url`; encoded separators are rejected.
- `canonicalize` yields `\\?\` paths and expands 8.3 names (`RUNNER~1`) that Node keeps, which broke Pi trust and Node module resolution. `platform::paths::canonical_for_cli` converts for external CLIs and Node; real file access keeps the original path.
- Post-install discovery also reads the User / Machine registry PATH, the IDE's PATH first (`platform/environment.rs`).
- Agents run in a kill-on-close Job Object, so owned descendants die with the agent or host; below normal priority in a parent job capped by `agentCpuCap` / `ACPIRA_AGENT_CPU_CAP`, since parallel test runs under Defender starve the desktop in kernel time.
- Session lock takeover only targets a direct child of another sidecar, verified by native parent PID / EXE name (`lock_holder.rs`).
- ChatGPT bridge commands use raw CMD arguments and drain output after exit, since a helper can hold the pipe open.
- Remote / WSL: the backend OS decides command syntax; `agents.json` keeps local Windows and remote Linux config apart.
- Release sidecars use the MSVC target with a static CRT; Windows x64 / arm64 are in `scripts/release/sidecar-targets.mjs`.
- Remote access: the gateway runs from a logon task (or the HKCU Run key) with `--supervise`, a GUI-subsystem program so the task opens no console; its control channel is an owner-only pipe, and `devices/` and `apns/` get an owner-only inherited DACL (host-architecture.md, Remote access gateway).

## Regression checks

- `scripts/ci/test-windows.ps1` is the single entry for native tests and Clippy on Windows targets, run by push / PR CI and release CI. Compiling, or a GNU cross-build run over SSH, does not prove the MSVC release build.
- Native tests: `tests/windows_launch.rs`, `tests/windows_compat.rs`, `platform::*` unit tests, `test/devin-install.test.ps1`, `test/windowsTerminal.test.mjs`, `cargo test -p acpira-gateway` (its end-to-end test needs `pnpm install` first). `login_service_round_trip` (`--ignored`) registers a real logon task.
- On Windows, Rust engine tests locate the repo through `canonical_for_cli` and the fake agent loader is passed as a file URL; `\\?\` and `S:\…` paths otherwise fail Node.

## Open gaps

- CMD `%*` forwarding shims truncate an argument with embedded quotes followed by `& % ! ^` characters. Global double escaping would break batch files reading `%~1`; a fix needs to tell forwarding shims apart. The built-in login argument shapes do not hit it.
- Some native Rust engine tests and the POSIX-only `pnpm test` cases (file modes, `fileLock`, `acpira serve` socket, bridge CLI) fail on Windows and are not addressed.
- Unverified: real OAuth per vendor, terminal TTY interaction, IDE button and browser-callback acceptance.
- Unverified: native arm64 execution; how each vendor's Windows CLI launches a stdio MCP command.
- Unverified for remote access: the firewall prompt on the fixed gateway path, `DnsServiceRegister` seen from an iPhone, Modern Standby with the power request held.
- Unverified: shared drives, redirected Known Folders, enterprise restrictions / antivirus, terminals with PowerShell or WSL interop disabled.
- Desktop Commander process detection reports `unknown` on Windows; presence of an executable is not shown as connected.
- Claude on Windows may pick up a user-level `ANTHROPIC_API_KEY`, so "not signed in" in Acpira does not mean the CLI lacks credentials; handling is in `agent-quirks.md`.
