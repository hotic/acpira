import { build } from 'esbuild';
import { fileURLToPath } from 'node:url';

// The main process and the preload, each one CommonJS file under dist/. The preload runs sandboxed, so it may require
// nothing but electron; packages/shared and packages/engine-client come from the repository through the same alias as the extension
const shared = fileURLToPath(new URL('../../packages/shared', import.meta.url));
const engineClient = fileURLToPath(new URL('../../packages/engine-client', import.meta.url));
const common = { bundle: true, platform: 'node', target: 'node22', format: 'cjs', external: ['electron'], alias: { '@shared': shared, '@engine-client': engineClient }, logLevel: 'info', sourcemap: 'linked' };

await Promise.all([
  build({ ...common, entryPoints: ['src/main.ts'], outfile: 'dist/main.cjs' }),
  build({ ...common, entryPoints: ['src/preload.ts'], outfile: 'dist/preload.cjs' }),
]);
