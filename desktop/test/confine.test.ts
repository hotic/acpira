import { mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { confine } from '../src/confine';

describe('confine', () => {
  const dirs: string[] = [];
  afterEach(() => { for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true }); });
  const setup = () => {
    const base = realpathSync(mkdtempSync(join(tmpdir(), 'acpira-confine-')));
    dirs.push(base);
    const root = join(base, 'root');
    mkdirSync(join(root, 'assets'), { recursive: true });
    writeFileSync(join(root, 'main.js'), '');
    writeFileSync(join(root, 'assets', 'a b.css'), '');
    writeFileSync(join(base, 'secret.txt'), '');
    symlinkSync(join(base, 'secret.txt'), join(root, 'escape.txt'));
    return { base, root };
  };

  it('serves files under the root, percent-escapes decoded', () => {
    const { root } = setup();
    expect(confine(root, 'main.js')).toBe(join(root, 'main.js'));
    expect(confine(root, 'assets/a%20b.css')).toBe(join(root, 'assets', 'a b.css'));
  });

  it('refuses parent segments, escaped ones, symlinks out, the root itself, missing files and bad escapes', () => {
    const { root } = setup();
    for (const p of ['../secret.txt', '%2e%2e/secret.txt', 'assets/../../secret.txt', 'escape.txt', '', '.', 'missing.js', '%E0%A4%A', 'a%00b']) {
      expect(confine(root, p), p).toBeUndefined();
    }
  });
});
