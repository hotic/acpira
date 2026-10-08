# Windows compatibility review

Review date: 2026-10-02; source baseline Acpira 1.8.0. Scope covers the platform boundaries of VS Code / Cursor, the IDEA backend, the Rust sidecar and the built-in agents. Native Windows runs used offline fixtures over SSH: no model calls, no real OAuth, and no rewriting of existing accounts or CLI stores.

## Platform boundaries

| Boundary | Finding and handling | Verified by |
|---|---|---|
| Executable lookup | On Windows the same-named npm POSIX shim is skipped in favour of EXE / COM / CMD / BAT; batch files go through the platform layer's shared `platform::command::spawn_spec` | `tests/windows_launch.rs`; registry regressions |
| Install and terminal auth | The shared `platform/terminal.rs` builds UTF-16LE encoded commands reused by both IDEs; TypeScript only delivers the launch arguments. Install scripts no longer go through a nested `-Command`; native argv is passed via `ProcessStartInfo`, keeping the empty arguments and double quotes that PowerShell 5.1 drops | `platform::terminal`; `test/windowsTerminal.test.mjs` |
| Devin update | The official 3000.11.3 install script overwrites the running EXE in place. The old entry is kept first and restored on failure; a backup still in use is left for later cleanup | `test/devin-install.test.ps1`, including a running EXE |
| Devin accounts | Devin 3000.11.3 on Windows uses the Roaming AppData Known Folder and ignores XDG / APPDATA overrides. Import reads the real location; terminal login watches the CLI's normal login file and only accepts writes after the flow opened; cancelling does not delete that file. Before and after querying the identity the global key is checked against the target, so no other saved account gets mislabelled | `accounts::devin`; read-only observation of the native CLI's `--version` / `auth status` |
| Codex account directory | `link_shared` used to skip outright on non-Unix. Both platforms now share the link primitive, exclude the account's own `auth.json` and keep files the account already has | `accounts::cli_home`, including deleting an account without deleting shared sessions |
| Shared config links | Hard links are recognised by file identity; directories use symlinks or native junctions, files fall back to hard links; `cmd /c mklink` is no longer run (it polluted sidecar stdout); dangling junctions can be replaced and cleaned up | `platform::files`; `shared_config::links` |
| Cross-process file lock | The old Windows `pid_alive` was always false, so a slow operation that was still alive could be taken over. It now checks the exit status through a native process handle and treats access denied as alive | `store::file_lock`, including an expired but live lock holder |
| Atomic writes | The standard library's tmp + rename is kept instead of a custom Windows overwrite; a regression covers overwriting an existing file | `store::file_lock::tests::atomic_write_replaces_existing_content` |
| File URLs | Drive letters, UNC, `\\?\`, CJK and reserved characters are handled in one place; decoding does not treat query / fragment as part of the file name and rejects encoded separators; a UTF-8 non-boundary slice on CJK paths was removed | `platform::file_url`; `normalize::local_path_tests` |
| Markdown file navigation | Literal file names like `%23L12` are kept; UNC directories are recognised; invalid percent encoding no longer throws during render | `test/fileLinks.test.ts` |
| Post-install discovery | The User / Machine registry PATH is read in addition, the IDE's original PATH keeps priority, entries are deduplicated by Windows rules; sessions, identity probes and terminals inherit the new PATH | `login_path`; `platform/environment.rs` |
| npm version diagnostics | CMD package lookup covers both the global directory and the project's `node_modules/.bin`, so the engine version and missing-platform-package hints are not missed | `adapter_info::tests::cmd_shims_find_both_global_and_project_local_packages` |
| Agent lifecycle | Reproduced leftover helpers after a natural exit; agents are now placed in a non-inherited kill-on-close Job Object before resuming, and natural exit / explicit kill / host crash all clean up owned descendants while unrelated processes survive | `platform/windows_process.rs`; `tests/windows_compat.rs` |
| Session lock takeover | Recognition used to be always false on Windows and kill a no-op; it now reads the native parent PID / EXE name, only takes over a direct child of another sidecar, and holds the handle and re-checks ownership before killing | `lock_holder.rs`; `takeover_only_targets_an_agent_of_another_sidecar` |
| ChatGPT bridge commands | Reproduced failures with quoted paths and a helper holding the output pipe so the command never finished; the shell source uses raw CMD arguments, reuses the Job Object, and drains output fully after exit | Windows-native cases in `external::chatgpt_cli::tests` |
| Pi project trust / external cwd | Reproduced a mismatch between Rust's `\\?\` prefix and Pi's Node path keys; conversion to a plain drive / UNC path happens only for external CLI config and the canonical cwd retry, actual file access keeps its original semantics | `platform::paths`; `shared_config::pi_trust`; comparison against Node's native realpath |
| Remote / WSL | The execution platform is the backend OS. The machine-level `agents.json` keeps local Windows and remote Linux config apart; the Windows client does not decide remote command syntax | `store/agent_config.rs`; `test/legacyAgents.test.ts` |
| Install archives and packaging | native-release checks digest / ZIP paths / CRC / install lock; the platform list includes Windows x64 / arm64; the official MSVC package uses a static CRT | `native_release` tests; `scripts/sidecar-targets.mjs`; release workflow |

## Native runs and automated regressions

- Windows host: Windows 11 (OS build 26200), PowerShell 5.1.26100.8115, Node 24.13.0, Devin 3000.11.3.
- The offline installer tests passed on native Windows, including a running EXE; EXE / CMD tests for all nine built-in login argument shapes passed too.
- Forcing Legacy argument mode in PowerShell 7.6.6 on macOS reproduced the native loss of empty arguments and corrupted double quotes; the shared launcher bypasses that marshalling and the regression passes.
- Native Windows x64 Rust regressions: host 121, shared 22, ACP launch 3, lifecycle 4, 150 passed in total; 1 helper-process entry ignored by design. Built by cross-compiling the GNU target on macOS and running the Windows EXEs over SSH; this does not replace CI for the MSVC release build.
- Registry PATH and the Roaming Known Folder use read-only native APIs and match what Windows' own APIs return; Pi paths match Node 24.13.0's real `realpathSync`.
- `scripts/test-windows.ps1` is the single entry point, covering native tests and Clippy for every Windows target; a dedicated Windows CI runs on push / PR and the release calls the same entry. Compiling only, or registering in CI, does not count as proof of native execution.

## Repository checks for this round

| Command | Result |
|---|---|
| `pnpm typecheck` | Passed |
| `ACPIRA_TEST_POWERSHELL=/tmp/acpira-devin-pwsh/pwsh pnpm test` | 61 suites, 473 tests passed |
| `pnpm build` | Passed; the existing CSS `::highlight` and large-bundle warnings remain |
| `RUST_TEST_THREADS=4 ACPIRA_TEST_POWERSHELL=/tmp/acpira-devin-pwsh/pwsh cargo test --workspace` (`rust/`) | 623 passed; 4 platform / fixture helper entries ignored by design |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` (`rust/`) | Passed on macOS; all targets also pass for `x86_64-pc-windows-gnu` |
| `./gradlew test` (`idea/`) | Passed in the previous review; this round's terminal restructuring did not touch Kotlin, so it was not rerun |
| Native Windows regressions | 150 Rust tests pass at default concurrency; the PowerShell 5.1 install fixture and the nine EXE / CMD login argument shapes pass |
| Leftover processes | No orphaned `fake-agent.ts` on macOS; no leftover fixture processes from this round on Windows |

Two full Rust runs at default concurrency each hit a failure in the existing Steer / Fork timing tests; they pass when rerun alone, and the full suite passes at lower test concurrency. The fake agent's `slow` turn only has a fixed 50 × 40 ms window, so that flakiness still needs its own fix; this round did not change the related logic or lengthen test waits to hide the failure.

Recheck after consolidating the terminal boundary: Windows script generation lives only in Rust, and `platform/` no longer references `acp::agents`. Generic command and argument escaping tests moved into `platform::command`; install source and the EXE / CMD execution tests for the nine login argument shapes moved into `platform::terminal`; TypeScript only checks launch argument delivery and POSIX behaviour. The first full Rust run read empty bytes in `agent_emitted_images_land_in_the_blob_store` because the test only waits for the file to appear while the write is async; it passed rerun alone and in the full workspace, with no change to the logic or the test. Windows terminal delivery, the offline Devin install and the leftover-process check also passed.

## Coverage limits

- New combined stress inputs added during the terminal boundary consolidation exposed an existing CMD limitation (2026-10-02, Windows 11 / PowerShell 5.1.26100.8115 / Node 24.13.0): when an argument with embedded double quotes is followed by a single argument like `& %ACPIRA_LITERAL_TEST% !bang! ^`, the second parse in a `%*` shim truncates the argument and tries to run the rest as a command. `spawn_spec` output is byte-identical before and after the migration, and native execution fails both ways; double escaping fixes such a shim but passes an extra `^"` to ordinary batch files that read `%~1`. The consolidation keeps the existing rule; the gap is not fixed yet and needs a clear split between forwarding shims and batch files that read their own arguments, not a global extra escaping layer. The nine built-in login argument shapes do not hit this combination.
- Running the Rust engine suites directly on this machine (2026-10-07, Windows 11 / Node 24.13.0, repo on `S:`): the `\\?\S:\…` from `canonicalize` made node fail to resolve the main module (`EISDIR lstat 'S:'`), and `--import S:\…\loader.mjs` was read as an `s:` URL scheme, so no fake agent could start. `tests/engine/support.rs` now locates the repo with `platform::paths::canonical_for_cli`, and `FakeAgent::loader_arg` passes a file URL on Windows. After that, 18 Windows-only failures remained in the full run (registry extensionless candidates, the verbatim spelling of `/tmp`, image / attachment paths, Devin terminal login, …), not addressed this round. A rerun on 2026-10-08 showed 17, of which `remembered_choices…` and `a_locked_credential_store…` are wait timeouts at full concurrency and pass alone. Native `pnpm test` on the same machine has 19 failures, all in POSIX-only cases: `secrets.json` mode 600, executable-bit restore, multi-process `fileLock`, the `acpira serve` socket persistent engine, real ChatGPT bridge CLI execution, `sidecarRuntime` re-probing on window focus, …; also not addressed.
- The offline SSH tests cover processes, arguments, environment, filesystem and protocol handshake; real browser OAuth for each vendor was not done, and they do not replace acceptance of the buttons, terminal interaction and browser callback inside both IDEs.
- Windows arm64 packaging and target definitions were reviewed, but the native execution host is x64; an arm64 host is still needed.
- Local MCP path resolution and config merging are verified; how each vendor's Windows build launches the stdio MCP command it receives has not been checked against each real CLI.
- Without Developer Mode / symlink privilege, file hard links are limited to the same volume and do not follow the source once it is atomically replaced; directory junctions are not subject to that file hard-link limit. Existing account files are kept, and account-private config is not force-overwritten.
- Actual permissions on shared drives, redirected Known Folders, enterprise process restrictions / antivirus, and custom terminals with PowerShell or WSL interop disabled still need acceptance in matching environments.
- Desktop Commander's process detection on Windows still reports `unknown`; the presence of an executable / config is not passed off as connected or paired.
- The changes are in the working tree and not yet on the Marketplace; an installed 1.8.0 does not get this round's fixes automatically.

## Short-path regression in release CI (2026-10-02)

The GitHub Windows runner's temp directory contains `RUNNER~1`. Node 22's plain `fs.realpathSync` keeps the short path while Rust's `std::fs::canonicalize` expands it to `runneradmin`, which failed the Node path comparison and the Pi parent-directory trust tests. A freshly created isolated directory on Windows 11 / Node 24.13.0 reproduces it too: plain realpath keeps `ACPIRA~1.1-S`, native realpath expands to the long name. `platform::paths::canonical_for_cli` resolves symlinks / junctions per component and keeps the case and 8.3 spelling of ordinary paths; Pi trust and the native session cwd retry share this boundary. New regressions compare against Node's output for short paths, case, parent-directory collapsing and junctions.

Fix verification: 623 tests pass in the macOS Rust workspace; all-target Clippy passes on macOS and for the Windows GNU target; on Windows 11 / Node 24.13.0, 144 library tests pass with 1 helper-process entry ignored. The new cases call the Windows short-path API explicitly and cover short names, case, parent-directory collapsing and junctions against real Node realpath. This still does not replace MSVC release CI for the fixed version.

## Claude auth state and silent retries (2026-10-07)

A real session log on Windows 11 / claude-agent-acp 0.84.0 / Claude Code 2.1.284 showed 7 × `401 API key is invalid`. The user-level `ANTHROPIC_API_KEY` was still picked up by the CLI, so the "not signed in" text shown when no Acpira account is saved does not mean the CLI lacks credentials. The adapter ignores auth warnings inside `api_retry`, so the UI showed no error during the long wait. Fix details are in `agent-quirks.md`: an explicit unauthenticated state is caught at startup, consecutive 401s end the turn, and the empty account list now reads "No accounts yet".

Native Windows x64 acceptance passed: a temporary smoke program built for the GNU target calls the installed `.cmd` adapter with isolated config and credential directories, clears auth environment only inside the test process, and reaches `auth_required` without sending a prompt; an offline Node fixture replaying consecutive 401s ends the running state and keeps the existing output. The user's installed extension was not replaced, user-level environment variables were not changed, and no real model requests were sent; this does not replace the official MSVC release build.
