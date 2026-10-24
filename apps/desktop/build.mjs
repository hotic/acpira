import { build } from 'esbuild';

// The main process and the preload, each one CommonJS file under dist/. The preload runs sandboxed, so it may require
// nothing but electron; @acpira/shared and @acpira/engine-client are workspace packages bundled in from their sources
const common = { bundle: true, platform: 'node', target: 'node22', format: 'cjs', external: ['electron'], logLevel: 'info', sourcemap: 'linked' };

await Promise.all([
  build({ ...common, entryPoints: ['src/main.ts'], outfile: 'dist/main.cjs' }),
  build({ ...common, entryPoints: ['src/preload.ts'], outfile: 'dist/preload.cjs' }),
]);
