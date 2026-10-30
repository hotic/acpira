// The platforms the Rust sidecar ships for. `platform` is the directory under dist/sidecar/ and, unchanged, under the IntelliJ
// plugin's sidecar/bin/; `vsce` lists the VS Code targets that package that binary (the musl build also serves Alpine).
// `gateway`: acpira-gateway, the paired-remote process, ships beside the engine, which looks for it there (acpira-host
// devices/)
export const SIDECAR_TARGETS = [
  { platform: 'mac-arm64', triple: 'aarch64-apple-darwin', vsce: ['darwin-arm64'], gateway: true },
  { platform: 'mac-x86_64', triple: 'x86_64-apple-darwin', vsce: ['darwin-x64'], gateway: true },
  { platform: 'linux-arm64', triple: 'aarch64-unknown-linux-musl', vsce: ['linux-arm64', 'alpine-arm64'], gateway: true },
  { platform: 'linux-x86_64', triple: 'x86_64-unknown-linux-musl', vsce: ['linux-x64', 'alpine-x64'], gateway: true },
  { platform: 'windows-arm64', triple: 'aarch64-pc-windows-msvc', vsce: ['win32-arm64'], gateway: true },
  { platform: 'windows-x86_64', triple: 'x86_64-pc-windows-msvc', vsce: ['win32-x64'], gateway: true },
];

export const exeName = platform => (platform.startsWith('windows-') ? 'acpira.exe' : 'acpira');
// The gateway's Cargo binary (what `cargo build --bin` takes) and its file on that platform
export const GATEWAY_EXE = 'acpira-gateway';
export const gatewayName = platform => (platform.startsWith('windows-') ? `${GATEWAY_EXE}.exe` : GATEWAY_EXE);

// The platform directory of the machine running the script
export function hostPlatform() {
  const os = { darwin: 'mac', linux: 'linux', win32: 'windows' }[process.platform];
  const arch = { arm64: 'arm64', x64: 'x86_64' }[process.arch];
  if (!os || !arch) throw new Error(`no sidecar target for ${process.platform}-${process.arch}`);
  return `${os}-${arch}`;
}

export function targetFor(platform) {
  const t = SIDECAR_TARGETS.find(x => x.platform === platform);
  if (!t) throw new Error(`unknown sidecar platform ${platform}; one of ${SIDECAR_TARGETS.map(x => x.platform).join(', ')}`);
  return t;
}
