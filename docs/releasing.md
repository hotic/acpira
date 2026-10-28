# Publishing Acpira

Publishing a stable GitHub Release triggers `.github/workflows/release.yml`. The `sidecar` job builds the Rust sidecar for six platforms on macOS, Linux (x64 and arm64) and Windows runners, runs the Rust unit tests and checks each runner's own binary. The `build` job checks the tag against `package.json`, installs locked dependencies, runs type checks and tests, runs the sidecar contracts against the shipped Linux binary, and packages eight platform VSIXes (`acpira-<version>-<target>.vsix`, darwin / linux / alpine / win32 × x64 / arm64, each with its binary). There is no universal VSIX: other platforms (`linux-armhf`, `web`) cannot install the extension. The macOS sidecars also carry `acpira-gateway`, the paired-remote process the engine looks for beside itself (`gateway` in `scripts/release/sidecar-targets.mjs`): the darwin VSIXes ship it in `bin/` and the desktop app as `Contents/MacOS/acpira-gateway`; the IntelliJ zips do not yet. It then builds the IntelliJ plugin at the same version (`apps/intellij/build.gradle.kts` reads `package.json`): tests, Plugin Verifier, the universal Marketplace zip and six per-platform zips (`acpira-<version>-<os>-<arch>.zip`, each with its sidecar binary). Separate jobs publish every VSIX to Visual Studio Marketplace and Open VSX, upload the universal zip to the JetBrains Marketplace, and attach everything, with SHA-256 checksums (`vsix.sha256`, `SHA256SUMS`), to the GitHub Release.

## One-time credentials

Add these repository secrets under [Settings → Secrets and variables → Actions](https://github.com/hotic/acpira/settings/secrets/actions). Never commit tokens or paste them into issues, release notes, or workflow inputs.

| Secret | Source and required access |
| --- | --- |
| `VSCE_PAT` | Azure DevOps personal access token for the Microsoft account that owns the `hotic` Marketplace publisher; Marketplace → Manage permission, with the organization selection required by the [VS Code publishing documentation](https://code.visualstudio.com/api/working-with-extensions/publishing-extension). |
| `OVSX_PAT` | [Open VSX access token](https://open-vsx.org/user-settings/tokens) for the account that signed the publisher agreement and can publish under `hotic`. |
| `JETBRAINS_MARKETPLACE_TOKEN` | [JetBrains Marketplace permanent token](https://plugins.jetbrains.com/author/me/tokens) of the account that owns plugin `com.github.hotic.acpira`. |

Tokens expire or can be revoked. Update the corresponding secret when rotating one. Microsoft currently documents retirement of global Azure DevOps PATs on December 1, 2026; migrate the Marketplace job to Microsoft Entra ID before that date. The current workflow uses PATs, not Entra ID or Open VSX trusted publishing.

The JetBrains Marketplace API requires an existing plugin listing. Subsequent releases upload only `acpira-<version>-universal.zip`, whose plugin version is the unsuffixed `X.Y.Z`. It carries all six sidecar binaries under `sidecar/bin/<os>-<arch>/`; only the backend selects and runs its matching one. The same artifact therefore works on a macOS Client connected to a Linux host.

OS/CPU dependencies inside optional content modules do not restrict compatibility of the whole plugin. Uploading six such versions would make them compete for installation. The six smaller platform archives are retained as GitHub downloads for manual installation; choose the host's archive for the backend and the client's archive for the frontend. The runtime-free base ZIP is a development artifact. See [Plugin Management in Split Mode](https://plugins.jetbrains.com/docs/intellij/plugin-management-in-split-mode.html).

A version still in review may not appear in the public listing and a repeated upload is refused as a duplicate. Read the job log before retrying. `scripts/release/verify-idea-packages.py` checks versions, module descriptors, binary consistency and permissions for every distribution before upload.

Creating the Open VSX namespace does not verify ownership. Follow [Namespace Access](https://github.com/eclipse-openvsx/openvsx/wiki/Namespace-Access) to claim exclusive publishing rights.

## Release an update

1. Run `pnpm version:set X.Y.Z`, which writes the version into `package.json`, the extension manifest `apps/vscode/package.json`, `rust/Cargo.toml` (`[workspace.package]`) and the desktop app's `apps/desktop/Cargo.toml` and `tauri.conf.json`, and runs `cargo update --workspace` in both Cargo workspaces so their `Cargo.lock` files follow (CI builds with `--locked`; `test/rustWorkspace.test.ts` checks them all), update `CHANGELOG.md`, then commit and push the release changes.
2. Create a GitHub Release whose tag exactly matches the manifest version, for example `v1.0.1` for version `1.0.1`. Select the commit containing the release changes. Publish it as a stable release.
3. Check the jobs in [Publish extension](https://github.com/hotic/acpira/actions/workflows/release.yml). Green build status alone does not confirm marketplace publication.
4. Confirm the version on [Visual Studio Marketplace](https://marketplace.visualstudio.com/items?itemName=hotic.acpira), [Open VSX](https://open-vsx.org/extension/hotic/acpira) and the [JetBrains Marketplace](https://plugins.jetbrains.com/plugin/com.github.hotic.acpira) (the unsuffixed `<version>` update). Cursor discovery can lag Open VSX publication because its marketplace proxy performs additional checks. Installed copies update according to each editor's automatic-update settings.

Drafts, standalone tag pushes, and prereleases do not publish to the marketplaces. Create the Release in the GitHub UI or using a personal `gh` login; a Release created by another workflow's default `GITHUB_TOKEN` does not trigger this workflow.

## Desktop app

The `desktop` job (macOS runner) builds the Tauri app with the release's `mac-arm64` sidecar, signs it with a Developer ID Application identity, notarizes and staples it (`apps/desktop/scripts/package.mjs`), checks it with `codesign --verify --deep --strict`, `spctl` and `stapler validate`, then packs it for the app's own updater with `acpira-update-pack` (`docs/dev/desktop.md` "Updates"): the full package `Acpira-<version>-darwin-aarch64.app.tar.zst`, a delta from each of the last three stable releases that carry a full package, and its part of the manifest. The `desktop-windows` job (Windows runner, no secrets) builds the portable `Acpira.exe` (`apps/desktop/scripts/package-windows.mjs`, unsigned for now) and packs it the same way: `Acpira-<version>-windows-x86_64.zip` for downloads, the full package `Acpira-<version>-windows-x86_64.exe.tar.zst`, its deltas and its manifest part. With publishing on each attaches its files (the dmg, the zip, the packages, `desktop.sha256` / `desktop-windows.sha256`) to the Release. The `desktop-update` job then merges the parts (`acpira-update-pack merge`) into `desktop-update.json` with the release body as its notes, signs it (Ed25519, `desktop-update.json.sig`) and attaches both last, so installed apps (which read `releases/latest/download/desktop-update.json`) never see an update whose files are missing. Without the Apple secrets the macOS job skips every step after its check and the manifest carries Windows alone; without `DESKTOP_UPDATE_KEY` no manifest is published. The extension and plugin releases never wait on these jobs; a latest Release without a manifest only makes installed apps' checks fail until the next one has it.

One-time setup, besides the secrets:

1. `cd rust && cargo run -p acpira-desktop --bin acpira-update-pack -- keygen`: the line on stdout is the secret seed (the `DESKTOP_UPDATE_KEY` secret, never in the repository or a log), the public key on stderr goes into `update::PUBLIC_KEYS` in `rust/crates/acpira-desktop/src/update/mod.rs`. A rotation lists the new key beside the old one for at least one release before the old one is dropped.
2. Put the Developer ID team (the certificate's OU) into `update::TEAM_ID`. Until both constants are set a macOS build never checks for updates (its About page says so); a Windows build needs only the key.

Both constants are set. Keep a copy of the seed outside the repository, such as in the release machine's keychain. `APPLE_CERTIFICATE` holds the Developer ID identity alone; check it by importing it into an empty keychain, as the job does. The App Store Connect API key (`APPLE_API_KEY` / `APPLE_API_ISSUER` / `APPLE_API_KEY_P8`) uses the App Manager role.

| Secret | Source |
| --- | --- |
| `APPLE_CERTIFICATE` | The Developer ID Application certificate with its private key, exported from Keychain Access as `.p12`, base64-encoded. |
| `APPLE_CERTIFICATE_PASSWORD` | The `.p12` export password. |
| `APPLE_API_KEY` / `APPLE_API_ISSUER` | Key ID and Issuer ID of an App Store Connect API key (Users and Access → Integrations) for notarization. |
| `APPLE_API_KEY_P8` | The contents of that key's `AuthKey_<id>.p8`. |
| `DESKTOP_UPDATE_KEY` | The hex seed from `acpira-update-pack keygen`. |

A retry keeps the assets already attached, like the other jobs; a changed app needs a new version.

## Validate or retry

IntelliJ release builds rerun Gradle tasks with the build cache disabled. Dependency downloads remain cached, but production and test classes are compiled together from the selected tag so inlined Kotlin constants cannot come from different builds.

Release builds and Windows compatibility checks use Rust 1.99.1. Update both workflow toolchain inputs together after validating a newer compiler and Clippy against all targets; do not use a moving stable toolchain for a release retry.

Use **Run workflow** with `ref: main` and publishing unchecked to run checks and download the resulting `extension-vsix` and `intellij-plugin` artifacts without publishing. This requires no marketplace secrets.

If one marketplace fails, fix its credential or the reported error and choose **Re-run failed jobs**. The VS Code publishers use `--skip-duplicate` and the JetBrains job skips the universal version when it is already listed, so an already-published version is not uploaded again. Existing GitHub Release assets are also retained. These retries do not replace an existing version; package changes require a new version and tag.

To publish an existing stable Release manually, use **Run workflow**, set `ref` to its exact tag, and check the publishing option. The Release must already exist and its tag must match the version at that commit. This also supports a Release created before the workflow was installed.
