# IntelliJ plugin (`apps/intellij/`)

A modular Kotlin shell (frontend tool window + JCEF, backend sidecar, native platform RPC) around the Rust sidecar; session state, ACP, transcripts and accounts live in the sidecar. Run `pnpm build` and `pnpm build:sidecar` first: the frontend takes `dist/webview/*` as resources and the sandbox needs this machine's binary (or `-PsidecarBin`). The IDE sends `System.out` and sidecar stderr into each sandbox's `log/idea.log`, not the Gradle output.

## Platform and build

- Built against 2026.1 (branch 261, JDK 21, `sinceBuild = 261`, no `untilBuild`); 262 runs 261-compiled plugins. The platform ships Kotlin 2.3 metadata, so the Kotlin Gradle plugin must be on 2.3. `useInstaller = true`, since the SDK archive has no JCEF natives.
- Do not add `bundledPlugin("intellij.platform.ui.jcef")`: it does not exist on 261.
- Release builds pass `--rerun-tasks --no-build-cache`: a cached `compileTestKotlin` once failed a test against a changed constant (suspected stale inlined constant, not confirmed).

## Modules and JCEF loading

- Root `plugin.xml` holds metadata and `<content>` only; `shared` is required, frontend / backend / backend-terminal / browser-legacy / browser-modular are optional. Frontend never references JCEF classes; it owns the `browserProvider` extension point.
- `browser-legacy` (261, the `com.intellij.modules.jcef` alias) and `browser-modular` (262+, the public JCEF modules) compile the same `browser-runtime/` sources. On a 262 Client the alias alone threw `NoClassDefFoundError: JBCefApp`; never restore internal `CefDelegate` / `JBCefNativeBundleProvider` bypasses or delayed retries.
- Processes run with `EnvironmentUtil.getEnvironmentMap()` / `PathEnvironmentVariableUtil.findInPath`, never `System.getenv` alone: a Dock / Toolbox launch has no shell PATH (a terminal `runIde` cannot show the difference).

## Serving the webview

- **Custom scheme not viable**: under out-of-process JCEF (the 2026.1 default) `CefSchemeRegistrar.addCustomScheme` returns false and module scripts fail as cross-origin. `web/AcpiraScheme` instead registers an `https://acpira.local` handler on the running `CefApp`, which is a secure context (clipboard, module scripts, fetch).
- Handlers URL-decode, normalize and confine to slash-separated segments (`..`, `%2e%2e`, `%2f` → 403); never `Paths.get()`, which breaks on Windows. Backend `BlobStore` resolves symlinks and requires a regular file under the real sessions root.
- Bridge: the served `index.html` splices `JBCefJSQuery.inject` in as `window.__acpiraApi.postMessage` before any script; Kotlin → page passes Base64 that the page parses, so no JSON is pasted into a JS literal.
- `window.open` without a user gesture never reaches `onBeforePopup` (Chromium's popup blocker), so it proves nothing about popup refusal.

## Frontend ↔ backend RPC

- `rpc/AcpiraBackendApi` + `connection/AcpiraConnection`: host events keep source order; each view's post channel stays gated until the attach flow emits its first state, so page `ready` cannot overtake attach. A later backend `READY` reloads an initialized page so it sends `ready` again.
- Toast / external / editor requests ride a second durable `uiRequests` flow to `FrontendUi`; `ProjectRemoteTopic` exists on 261 but its listener API is internal and fails Plugin Verifier.

## Sidecar service

- `sidecar/SidecarService`, one per project. `SidecarLocator`: `ACPIRA_SIDECAR_BIN` / `-PsidecarBin`, else the packaged `sidecar/bin/<os>-<arch>/acpira[.exe]` for the backend's OS / CPU (exec bit restored). No Node fallback; a missing binary is a `SidecarSetupException` in the error panel.
- Start, handshake, attach, outbox drain and sends share one single-thread executor and a generation number: two views cannot spawn two processes, `ready` cannot overtake `attachView`, and a replaced process's callbacks cannot touch the new one.
- `helloOk` does not reset the restart counter (only a >60 s run or Retry does), so a crash-on-start loop still ends in `FAILED`. IDE exit → `shutdown` leaves no orphan agents.

## Backend services

- `settings/AcpiraSettings` stores the flat `acpira.*` JSON as one string in a `SimplePersistentStateComponent`; `SerializablePersistentStateComponent` over an immutable data class serialized to an empty `<State />` (pinned by `AcpiraSettingsStateTest`).
- `backend-terminal/IdeTerminalRunner`: `runInTerminal` exists only with the Terminal plugin. A `null` env value must unset the variable per shell dialect (`env -u`, `$env:X=$null`, `set "X="`), chosen from `TerminalProjectOptionsProvider.shellPath`.
- `platform/FileRank` ports `apps/vscode/src/fileRank.ts`; `FileRankTest` mirrors the VS Code test so both scorers stay identical.
- `editor/EditorRelay` sends `editorSelection` / `editorCopy`; `EditorRange.of` mirrors `apps/vscode/src/editorSelection.ts`. Only `isInLocalFileSystem` files are offered, so split-mode remote files are not. Unverified in a live IDE.

## Split mode (2026.1 / 262 Client)

- `runIdeSplitBoth`: exactly one sidecar, in the backend project. `runIdeSplitFrontend` must never start a local sidecar; it shows the backend-unreachable state after 15 s.
- JCEF and `AcpiraScheme` stay frontend-only: in `runIdeSplitBackend` the projected browser does not know the backend-registered factory (`ERR_UNKNOWN_URL_SCHEME`).
- File opening: backend sends absolute paths; the frontend calls `resolveFile` over RPC and the backend calls `rpcId()` inside that Client session. Calling `rpcId()` on the sidecar reader thread loses the session (`serializedValue=null`); resolving via frontend `LocalFileSystem` only works on one machine.
- Directories: `resolveFile` returns the backend's directory flag, since Client remote VFS may report directories as files; the frontend selects through `SelectInTarget.selectIn`, because `ProjectView.select` only expands remote roots.
- Split run tasks pass `-PjcefDebug` to the backend only (no CDP in the Client); `-PautoOpen` does not open the Client's tool window.

## Distribution

- Marketplace gets only the universal ZIP (all six sidecars; `SidecarLocator` picks): optional backend-module OS dependencies cannot route whole-plugin installs in split mode, so per-platform variants are manual downloads only.
- `verifyPlugin` checks the binary-free base ZIP (not installable alone); `scripts/release/verify-idea-packages.py` checks all archives. A Marketplace upload does not mean approval.

## Open questions

- Session links: the page gets no `sessionLinkBase`, so Copy link writes `acpira://session/<id>`, which the OS cannot open; there is no `jetbrains://` handler. Copy as Markdown under JCEF is unverified.
