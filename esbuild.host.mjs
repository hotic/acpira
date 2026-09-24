import { context } from 'esbuild';

// The VS Code shell: src/host/extension.ts → dist/extension.cjs (the extension host is CJS; the vscode module is provided by the host).
// The engine behind it is the Rust sidecar (rust/, `pnpm build:sidecar`)
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
]);

if (watch) await Promise.all(contexts.map(c => c.watch()));
else { for (const c of contexts) { await c.rebuild(); await c.dispose(); } }
