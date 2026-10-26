import { cpSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { constants as zlibConstants, zstdCompressSync } from 'node:zlib';

// The page as the packages ship it (scripts/package.mjs, scripts/package-linux.mjs): dist/webview copied to `out` with
// every file under assets/ zstd-compressed beside its name (`x.js` → `x.js.zst`, served decompressed by
// acpira_desktop::page::route), ~6.5 → ~1.8 MB on disk (2026-10-25). main.js and main.css stay plain: the updater patches
// them from the installed copy nearly every release, and a compressed file would turn that ~60 KB patch into ~650 KB.
// Chunk names carry a content hash, so a chunk only changes with its code
export function packPage(repo, out) {
  if (typeof zstdCompressSync !== 'function') {
    console.error(`node ${process.version} has no zlib zstd: packaging needs Node 22.15 or later`);
    process.exit(1);
  }
  rmSync(out, { recursive: true, force: true });
  cpSync(join(repo, 'dist', 'webview'), out, { recursive: true });
  for (const name of readdirSync(join(out, 'assets'))) {
    const file = join(out, 'assets', name);
    const params = { [zlibConstants.ZSTD_c_compressionLevel]: 19 };
    writeFileSync(`${file}.zst`, zstdCompressSync(readFileSync(file), { params }));
    rmSync(file);
  }
}
