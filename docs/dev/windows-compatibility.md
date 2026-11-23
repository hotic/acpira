# Windows compatibility

Windows boundaries the code works around, their checks, and what lacks native acceptance. Platform code: `rust/crates/acpira-host/src/platform/`.

## Platform boundaries

- Executable lookup skips the same-named npm POSIX shim; batch files always go through `platform::command::spawn_spec`.
- Terminal and install scripts are built only in Rust (`platform/terminal.rs`, `acp/agents/installer.rs`); native argv goes via `ProcessStartInfo` because PowerShell 5.1 drops empty arguments and mangles double quotes.
- Devin 3000.11.3's install script overwrites the running EXE in place, so the old entry is kept and restored on failure. Devin uses the Roaming AppData Known Folder, ignoring XDG / APPDATA overrides (`accounts::devin`).
- Shared config links: never `cmd /c mklink` (it writes to sidecar stdout). File hard links stay on one volume and stop following the source once it is atomically replaced.
- File lock liveness treats access denied as alive (`store::file_lock`).
- File URLs are handled only in `platform::file_url`; encoded separators are rejected.
- `canonicalize` yields `\\?\` paths and expands 8.3 names that Node keeps, breaking Pi trust and Node module resolution: `platform::paths::canonical_for_cli` converts for external CLIs and Node; real file access keeps the original path.
- Agents run in a kill-on-close Job Object under a CPU cap (`agentCpuCap`, `ACPIRA_AGENT_CPU_CAP`) and below-normal priority by default (`agentPriority`, `ACPIRA_AGENT_PRIORITY`), since parallel runs under Defender starve the desktop in kernel time; switching to `normal` avoids starvation on already-saturated hosts. Lifting an unset CPU cap must skip `SetInformationJobObject(JobObjectCpuRateControlInformation)` (Windows returns `ERROR_INVALID_PARAMETER`). `AgentProcess` `INIT_TIMEOUT` is 120 s to cover cold starts on loaded Windows machines.
- Session lock takeover only targets a verified direct child of another sidecar (`lock_holder.rs`).
- Remote / WSL: the backend OS decides command syntax; `agents.json` keeps local Windows and remote Linux config apart.
- Release sidecars: MSVC target, static CRT (`scripts/release/sidecar-targets.mjs`).
- Credential Manager items belong to a logon session: an SSH key session has none, so a keyring login reads as absent there (`platform::credentials`).
- `agy` 1.5.9: login in the generic credential `gemini:antigravity`; under the SSH variables it takes the token file as on macOS; print mode finds its home through USERPROFILE, not HOME. An account's gemini dir keeps its own `conversation_summaries.db`: hard-linked, it would get a second `-wal` / `-shm` (`accounts::antigravity`).
- Saved Droid logins run with `FACTORY_DISABLE_KEYRING=1` and keep the file pair (`accounts::droid`).
- Machine engine: an owner-only pipe named after the data root (`machine_engine::ensure::socket_path`). A pipe name lives while any instance is open, so a successor joins the drained engine's pipe once it holds the `run/` lock. Node cannot check a pipe server's user: VS Code trusts the name `acpira engine` just checked. Java pipe I/O runs one call at a time, so IntelliJ goes through `acpira engine --attach`. `acpira engine` keeps its std handles out of the engine it starts, else a shell reading its output waits for the engine to end.
- Builds up to 2.1.6 ran an engine per shell (desktop, remote access, each IDE window). The machine engine drains those holding a `run/` lock of its data root (`retire_legacy`); an IDE window's child process goes only when the window reloads on a newer build.
- One exe on Windows: anything that runs the engine by `current_exe()` (a local-run stand-in's hook, `tool-guard`) reaches `run` only through `cli::is_engine_command`. A subcommand missing there opens the app instead and prints nothing, which `agy` (1.5.13) reads as an allow: its native read then asks, the engine refuses and the turn stops silently.
- Remote access: the logon task runs the GUI-subsystem engine (`serve --service --log`, the exe's own output redirected) so no console opens; owner-only control pipe and DACLs (`host-architecture.md`, Remote access gateway). Unverified on Windows: `/Create /F` over a running instance, `SetStdHandle` redirection.

## Regression checks

- `scripts/ci/test-windows.ps1` is the single native entry (push / PR and release CI). Compiling or a GNU cross-build does not prove the MSVC release build.
- `login_service_round_trip` (`--ignored`) registers a real logon task; the gateway end-to-end test needs `pnpm install`.
- Rust engine tests locate the repo via `canonical_for_cli` and pass the fake agent loader as a file URL; `\\?\` and `S:\…` paths otherwise fail Node.

## Open gaps

- CMD `%*` forwarding shims truncate an argument with embedded quotes followed by `& % ! ^`. Global double escaping would break batch files reading `%~1`; a fix must tell forwarding shims apart. Built-in login arguments do not hit it.
- Some native Rust engine tests and POSIX-only `pnpm test` cases (file modes, `fileLock`, `acpira serve` socket) fail on Windows.
- Unverified: real OAuth per vendor, terminal TTY, IDE buttons and browser callbacks, native arm64, each vendor's stdio MCP launch, shared drives, redirected Known Folders, antivirus / enterprise policy, PowerShell or WSL interop disabled.
- Unverified for remote access: firewall prompt, `DnsServiceRegister` seen from an iPhone, Modern Standby.
- Unverified: a droid sign-in on Windows (a saved login's file pair, keytar's `Factory CLI/auth-encryption-key` item behind the CLI's own login; droid 1.16.0 was checked for `FACTORY_HOME_OVERRIDE`, `FACTORY_API_KEY` and its store code only).
- Claude may use a user-level `ANTHROPIC_API_KEY`, so "not signed in" does not mean the CLI lacks credentials (`agent-quirks-claude.md`).
