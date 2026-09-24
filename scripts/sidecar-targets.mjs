// The platforms the Rust sidecar ships for. `platform` is the directory under dist/sidecar/ and, unchanged, under the IntelliJ
// plugin's sidecar/bin/; `vsce` lists the VS Code targets that package that binary (the musl build also serves Alpine)
export const SIDECAR_TARGETS = [
  { platform: 'mac-arm64', triple: 'aarch64-apple-darwin', vsce: ['darwin-arm64'] },
  { platform: 'mac-x86_64', triple: 'x86_64-apple-darwin', vsce: ['darwin-x64'] },
  { platform: 'linux-arm64', triple: 'aarch64-unknown-linux-musl', vsce: ['linux-arm64', 'alpine-arm64'] },
  { platform: 'linux-x86_64', triple: 'x86_64-unknown-linux-musl', vsce: ['linux-x64', 'alpine-x64'] },
  { platform: 'windows-arm64', triple: 'aarch64-pc-windows-msvc', vsce: ['win32-arm64'] },
  { platform: 'windows-x86_64', triple: 'x86_64-pc-windows-msvc', vsce: ['win32-x64'] },
];

export const exeName = platform => (platform.startsWith('windows-') ? 'acpira.exe' : 'acpira');

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
