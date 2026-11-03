import { execFileSync, spawnSync } from 'node:child_process';
import { chmodSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, readlinkSync, rmSync, statSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';

// Run with LDAI_RUNTIME_FILE and Tauri's cached linuxdeploy output plugin on the Linux packaging machine.
const runtime = process.env.LDAI_RUNTIME_FILE;
const linuxTools = process.platform === 'linux' && !!runtime
  && ['mksquashfs', 'unsquashfs'].every(cmd => !spawnSync(cmd, ['-version'], { stdio: 'ignore' }).error);
const roots: string[] = [];
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
const helper = new URL('../apps/desktop/scripts/repack-appimage.mjs', import.meta.url).href;
const repack = (image: string, dir: string) => execFileSync(process.execPath, ['--input-type=module', '-e',
  `import { repackAppImage } from ${JSON.stringify(helper)}; repackAppImage(process.argv[1], process.argv[2]);`, image, dir], { encoding: 'utf8', stdio: 'pipe' });

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'acpira-appimage-test-'));
  roots.push(root);
  const dir = join(root, 'AppDir');
  mkdirSync(dir);
  writeFileSync(join(dir, 'AppRun'), '#!/bin/sh\necho packaged-app\n', { mode: 0o755 });
  writeFileSync(join(dir, 'page.js'), 'const page = "keep every resource";\n'.repeat(50_000));
  symlinkSync('page.js', join(dir, 'page-link.js'));
  writeFileSync(join(dir, 'Acpira.desktop'), '[Desktop Entry]\nName=Acpira\nExec=AppRun\nIcon=Acpira\nType=Application\nCategories=Development;\n');
  writeFileSync(join(dir, 'Acpira.png'), Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAADUlEQVR4nGP4z8AARAAI/gH/xp559wAAAABJRU5ErkJggg==', 'base64'));
  const header = readFileSync(runtime!);
  const offset = header.length;
  const payload = join(root, 'payload.sqfs');
  execFileSync('mksquashfs', [dir, payload, '-noappend', '-noD', '-processors', '1', '-no-progress'], { stdio: 'pipe' });
  const image = join(root, 'Acpira.AppImage');
  writeFileSync(image, Buffer.concat([header, readFileSync(payload)]));
  chmodSync(image, 0o755);
  return { root, dir, image, header, offset };
}

describe.skipIf(!linuxTools)('AppImage recompression', () => {
  it('preserves the runtime, files, links and executable modes through the actual appimagetool', () => {
    const { root, dir, image, header, offset } = fixture();
    const before = statSync(image).size;
    expect(repack(image, dir)).toContain('same runtime and files');
    expect(statSync(image).size).toBeLessThan(before);
    // appimagetool refreshes .digest_md5; the executable's ELF header and runtime version must stay intact.
    expect(readFileSync(image).subarray(0, 64)).toEqual(header.subarray(0, 64));
    expect(spawnSync(image, ['--appimage-version'], { encoding: 'utf8' }).stderr).toBe(spawnSync(runtime!, ['--appimage-version'], { encoding: 'utf8' }).stderr);
    expect(execFileSync(image, ['--appimage-extract-and-run'], { encoding: 'utf8' }).trim()).toBe('packaged-app');
    const unpacked = join(root, 'unpacked');
    execFileSync('unsquashfs', ['-no-progress', '-d', unpacked, '-o', String(offset), image], { stdio: 'pipe' });
    expect(readFileSync(join(unpacked, 'page.js'))).toEqual(readFileSync(join(dir, 'page.js')));
    expect(readlinkSync(join(unpacked, 'page-link.js'))).toBe('page.js');
    expect(statSync(join(unpacked, 'AppRun')).mode & 0o777).toBe(0o755);
    expect(execFileSync(image, ['--appimage-offset'], { encoding: 'utf8' }).trim()).toBe(String(offset));
    expect(readdirSync(root).some(name => name.startsWith('.repack-'))).toBe(false);
  });

  it('leaves the original intact and cleans staging after compression fails', () => {
    const { root, image } = fixture();
    const original = readFileSync(image);
    expect(() => repack(image, join(root, 'missing-AppDir'))).toThrow();
    expect(readFileSync(image)).toEqual(original);
    expect(readdirSync(root).some(name => name.startsWith('.repack-'))).toBe(false);
  });
});
