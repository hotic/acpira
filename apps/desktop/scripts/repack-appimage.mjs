import { spawnSync } from 'node:child_process';
import { chmodSync, closeSync, existsSync, mkdtempSync, openSync, readSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { availableParallelism, homedir } from 'node:os';
import { dirname, join } from 'node:path';

// Keep linuxdeploy's complete AppDir and runtime; only change the SquashFS compression. Its runtime does not support
// xz, and swapping it for an xz-capable runtime regresses mounted startup. Level 19 is almost as small as 22 here.
export function repackAppImage(image, appDir) {
  const run = (cmd, args, capture = false, cwd) => {
    const r = spawnSync(cmd, args, { cwd, encoding: 'utf8', stdio: capture ? 'pipe' : 'inherit', env: { ...process.env, ARCH: 'x86_64' } });
    if (r.status !== 0) throw new Error(`${cmd} failed: ${r.error?.message ?? (capture ? r.stderr : `exit ${r.status}`)}`);
    return r.stdout;
  };
  // Re-signing / regenerating update metadata belongs to the original packaging step, not a post-signing rewrite.
  if (process.env.LDAI_SIGN || process.env.SIGN || process.env.LDAI_UPDATE_INFORMATION || process.env.UPDATE_INFORMATION) {
    throw new Error('AppImage recompression must run before signing or generating update metadata');
  }
  const before = statSync(image);
  const offset = Number(run(image, ['--appimage-offset'], true).trim());
  if (!Number.isSafeInteger(offset) || offset <= 0 || offset >= before.size) throw new Error(`invalid AppImage offset: ${offset}`);
  const header = Buffer.alloc(offset);
  const source = openSync(image, 'r');
  try {
    if (readSync(source, header, 0, header.length, 0) !== header.length) throw new Error('truncated AppImage runtime');
  } finally {
    closeSync(source);
  }

  // Build beside the destination so a failed compression never damages the usable original.
  const temp = mkdtempSync(join(dirname(image), '.repack-'));
  const packed = join(temp, 'Acpira.AppImage');
  try {
    // Use the same appimagetool as Tauri, including its embedded-MD5 refresh and runtime-prefix handling. The optional
    // output plugin may be absent after a failed download; linuxdeploy carries a fallback copy (Tauri CLI 2.12.4).
    const cache = join(process.env.XDG_CACHE_HOME || join(homedir(), '.cache'), 'tauri');
    const plugin = join(cache, 'linuxdeploy-plugin-appimage.AppImage');
    run(existsSync(plugin) ? plugin : join(cache, 'linuxdeploy-07333c6-x86_64.AppImage'), ['--appimage-extract'], true, temp);
    const extracted = join(temp, 'squashfs-root');
    const tool = existsSync(join(extracted, 'usr/bin/appimagetool')) ? join(extracted, 'usr/bin/appimagetool')
      : join(extracted, 'plugins/linuxdeploy-plugin-appimage/usr/bin/appimagetool');
    const runtime = join(temp, 'runtime');
    writeFileSync(runtime, header);
    const options = ['-Xcompression-level', '19', '-b', '1M', '-processors', String(Math.min(4, availableParallelism()))];
    run(tool, ['--runtime-file', runtime, '--no-appstream', '--comp', 'zstd',
      ...options.map(value => `--mksquashfs-opt=${value}`), appDir, packed]);
    chmodSync(packed, before.mode & 0o777);
    if (Number(run(packed, ['--appimage-offset'], true).trim()) !== offset) throw new Error('repacked AppImage runtime offset changed');
    const after = statSync(packed).size;
    if (after < before.size) {
      renameSync(packed, image);
      console.log(`AppImage compression: ${(before.size / 1048576).toFixed(2)} → ${(after / 1048576).toFixed(2)} MiB (same runtime and files)`);
    } else {
      console.log('AppImage compression: keeping the smaller original');
    }
  } finally {
    rmSync(temp, { recursive: true, force: true });
  }
}
