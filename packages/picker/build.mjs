import { context } from 'esbuild';
import { fileURLToPath } from 'node:url';

// picker.ts → the repository's dist/picker.js: the element picker the engine's preview proxy serves to pages shown in the
// browser panel. It runs in those pages, so it is a plain browser script; every shell ships the same file
const watch = process.argv.includes('--watch');
const here = (path) => fileURLToPath(new URL(path, import.meta.url));

const ctx = await context({
  bundle: true,
  platform: 'browser',
  target: 'es2020',
  format: 'iife',
  sourcemap: watch,
  minify: !watch,
  logLevel: 'info',
  entryPoints: [here('./picker.ts')],
  outfile: here('../../dist/picker.js'),
});

if (watch) await ctx.watch();
else { await ctx.rebuild(); await ctx.dispose(); }
