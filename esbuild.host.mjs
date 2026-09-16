import { context } from 'esbuild';

// Host bundles: src/host/extension.ts → dist/extension.cjs (the extension host is CJS; the vscode module is provided by the host) and
// src/host/server.ts → dist/host-server.cjs (the sidecar a non-VS Code shell spawns; nothing in its graph may import vscode, so it is
// not marked external there and a stray import fails the build)
const watch = process.argv.includes('--watch');

const common = {
  bundle: true,
  platform: 'node',
  target: 'node22',
  format: 'cjs',
  sourcemap: watch,
  minify: !watch,
  logLevel: 'info',
  alias: { '@shared': './src/shared' },
};

const contexts = await Promise.all([
  context({ ...common, entryPoints: ['src/host/extension.ts'], outfile: 'dist/extension.cjs', external: ['vscode'] }),
  context({ ...common, entryPoints: ['src/host/server.ts'], outfile: 'dist/host-server.cjs' }),
  context({ ...common, entryPoints: ['src/host/external/chatgptCli.ts'], outfile: 'dist/chatgpt-bridge.cjs' }),
]);

if (watch) await Promise.all(contexts.map(c => c.watch()));
else { for (const c of contexts) { await c.rebuild(); await c.dispose(); } }
