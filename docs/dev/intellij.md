# IntelliJ plugin (`apps/intellij/`)

A modular Kotlin shell (frontend tool window + JCEF, backend sidecar, native platform RPC) around the Rust sidecar; session state, ACP, transcripts and accounts live in the sidecar. Run `pnpm build` and `pnpm build:sidecar` first. `System.out` and sidecar stderr go to each sandbox's `log/idea.log`, not the Gradle output.

## Platform and build

- Built against 2026.1 (branch 261, JDK 21, no `untilBuild`); 262 runs 261-compiled plugins. The platform ships Kotlin 2.3 metadata, so the Kotlin Gradle plugin must be 2.3. `useInstaller = true`: the SDK archive has no JCEF natives.
- No `bundledPlugin("intellij.platform.ui.jcef")`: it does not exist on 261.
- Release builds pass `--rerun-tasks --no-build-cache`: a cached `compileTestKotlin` once failed a test against a changed constant.

## Modules and JCEF loading

- Frontend never references JCEF classes; it owns the `browserProvider` extension point.
- `browser-legacy` (261, the `com.intellij.modules.jcef` alias) and `browser-modular` (262+, the public JCEF modules) compile the same `browser-runtime/` sources. On a 262 Client the alias alone threw `NoClassDefFoundError: JBCefApp`; never restore internal `CefDelegate` / `JBCefNativeBundleProvider` bypasses or delayed retries.
- Processes use `EnvironmentUtil.getEnvironmentMap()` / `PathEnvironmentVariableUtil.findInPath`, never `System.getenv` alone: a Dock / Toolbox launch has no shell PATH.

## Serving the webview

- Custom schemes are not viable: under out-of-process JCEF (the 2026.1 default) `CefSchemeRegistrar.addCustomScheme` returns false and module scripts fail as cross-origin. `web/AcpiraScheme` serves `https://acpira.local` from the running `CefApp` instead.
- Handlers confine to slash-separated segments (`..`, `%2e%2e`, `%2f` → 403); never `Paths.get()`, which breaks on Windows. `BlobStore` resolves symlinks and requires a regular file under the real sessions root.
- Kotlin → page passes Base64, never JSON pasted into a JS literal.
- `window.open` without a user gesture never reaches `onBeforePopup` (popup blocker): it proves nothing about refusal.

## Frontend, backend and sidecar

- Each view's post channel stays gated until the attach flow emits its first state, so page `ready` cannot overtake attach; a later backend `READY` reloads the page so it sends `ready` again.
- `ProjectRemoteTopic`'s listener API is internal on 261 and fails Plugin Verifier, hence the `uiRequests` flow.
- `SidecarService` (one per project): start, handshake, attach, outbox drain and sends share one single-thread executor and a generation number: two views cannot spawn two processes, and a replaced process's callbacks cannot touch the new one.
- `helloOk` does not reset the restart counter (only a >60 s run or Retry), so a crash-on-start loop still ends in `FAILED`.
- `AcpiraSettings` stores the `acpira.*` JSON as one string: `SerializablePersistentStateComponent` over an immutable data class saved an empty `<State />` (`AcpiraSettingsStateTest`).
- `IdeTerminalRunner`: a `null` env value must unset the variable in the shell's own dialect (`env -u`, `$env:X=$null`, `set "X="`).
- `FileRank` / `EditorRange` port `apps/vscode/src/fileRank.ts` / `editorSelection.ts`; their tests mirror VS Code's to keep both identical.

## Split mode (2026.1 / 262 Client)

- One sidecar, in the backend; the frontend never starts one.
- JCEF and `AcpiraScheme` stay frontend-only: a projected backend browser does not know the backend-registered factory (`ERR_UNKNOWN_URL_SCHEME`).
- File opening: the backend sends absolute paths, the frontend calls `resolveFile` over RPC, and the backend calls `rpcId()` inside that Client session. `rpcId()` on the sidecar reader thread loses the session (`serializedValue=null`); frontend `LocalFileSystem` works only on one machine.
- `resolveFile` returns the backend's directory flag (Client VFS may call directories files); selection uses `SelectInTarget.selectIn`, since `ProjectView.select` only expands remote roots.

## Distribution

- Marketplace gets only the universal ZIP (all six sidecars): optional backend-module OS dependencies cannot route split-mode installs; per-platform ZIPs are manual downloads.
- `verifyPlugin` checks the binary-free base ZIP (not installable alone); `scripts/release/verify-idea-packages.py` checks all archives. Upload is not approval.

## Open questions

- Session links: the page gets no `sessionLinkBase`, so Copy link writes `acpira://session/<id>`, which the OS cannot open; there is no `jetbrains://` handler. Copy as Markdown under JCEF: unverified.
