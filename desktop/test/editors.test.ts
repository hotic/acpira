import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { editorArgs, findEditor } from '../src/editors';

describe('editorArgs', () => {
  it('opens at the line in each editor\'s own form', () => {
    const at = { path: '/repo/a b.ts', line: 12 };
    expect(editorArgs('code', at)).toEqual(['-g', '/repo/a b.ts:12']);
    expect(editorArgs('cursor', at)).toEqual(['-g', '/repo/a b.ts:12']);
    expect(editorArgs('windsurf', at)).toEqual(['-g', '/repo/a b.ts:12']);
    expect(editorArgs('zed', at)).toEqual(['/repo/a b.ts:12']);
    expect(editorArgs('subl', at)).toEqual(['/repo/a b.ts:12']);
    expect(editorArgs('idea', at)).toEqual(['--line', '12', '/repo/a b.ts']);
  });

  it('opens the file alone without a line', () => {
    expect(editorArgs('code', { path: '/plan.md' })).toEqual(['-g', '/plan.md']);
    expect(editorArgs('idea', { path: '/plan.md' })).toEqual(['/plan.md']);
  });
});

describe('findEditor', () => {
  const home = '/Users/me';

  it('takes PATH in order before the app bundles', () => {
    const present = new Set(['/opt/b/code', '/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code']);
    expect(findEditor('code', { PATH: '/opt/a:/opt/b' }, 'darwin', home, p => present.has(p))).toBe('/opt/b/code');
  });

  it('falls back to the macOS bundle, under /Applications first, then ~/Applications', () => {
    const toolbox = join(home, 'Applications', 'IntelliJ IDEA Ultimate.app/Contents/MacOS/idea');
    expect(findEditor('idea', { PATH: '/usr/bin' }, 'darwin', home, p => p === toolbox)).toBe(toolbox);
    const both = new Set(['/Applications/Zed.app/Contents/MacOS/cli', join(home, 'Applications', 'Zed.app/Contents/MacOS/cli')]);
    expect(findEditor('zed', {}, 'darwin', home, p => both.has(p))).toBe('/Applications/Zed.app/Contents/MacOS/cli');
  });

  it('finds nothing outside PATH on Linux, and only .exe files on Windows', () => {
    expect(findEditor('code', { PATH: '/usr/bin' }, 'linux', home, p => p.startsWith('/Applications'))).toBeUndefined();
    const exe = join('C:\\Tools', 'zed.exe');
    expect(findEditor('zed', { PATH: 'C:\\Tools;C:\\Other' }, 'win32', home, p => p === exe)).toBe(exe);
    expect(findEditor('code', { PATH: 'C:\\VSCode\\bin' }, 'win32', home, p => p.endsWith('code.cmd'))).toBeUndefined();
  });
});
