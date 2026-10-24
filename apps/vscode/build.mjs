import { context } from 'esbuild';
import { fileURLToPath } from 'node:url';

// The VS Code shell: src/extension.ts → dist/extension.cjs, next to this manifest (the extension host is CJS; the vscode
// module is provided by the host). The engine behind it is the Rust sidecar (rust/, `pnpm build:sidecar`); the page and the
// picker it serves are the repository's shared dist/webview and dist/picker.js (packages/ui, packages/picker)
const watch = process.argv.includes('--watch');
const here = (path) => fileURLToPath(new URL(path, import.meta.url));

const ctx = await context({
  bundle: true,
  platform: 'node',
  target: 'node22',
  format: 'cjs',
  sourcemap: watch,
  minify: !watch,
  logLevel: 'info',
  entryPoints: [here('./src/extension.ts')],
  outfile: here('./dist/extension.cjs'),
  external: ['vscode'],
});

if (watch) await ctx.watch();
else { await ctx.rebuild(); await ctx.dispose(); }
