import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { acpiraKeys, ideSettingsPaths, stripJsonc } from '../src/vscodeSettings';

describe('VS Code settings import', () => {
  it('reads JSONC: comments and trailing commas go, strings stay as written', () => {
    const text = `\uFEFF{
      // a comment
      "acpira.defaultAgent": "claude", /* inline */
      "acpira.note": "keep // this, and /* this */ too",
      "acpira.escaped": "a \\"quoted\\" word,",
      "acpira.list": [1, 2,],
      "editor.fontSize": 13,
    }`;
    expect(JSON.parse(stripJsonc(text))).toEqual({
      'acpira.defaultAgent': 'claude',
      'acpira.note': 'keep // this, and /* this */ too',
      'acpira.escaped': 'a "quoted" word,',
      'acpira.list': [1, 2],
      'editor.fontSize': 13,
    });
  });

  it('takes the acpira.* keys without the prefix, minus the column settings and the per-machine agent list', () => {
    expect(acpiraKeys({ 'acpira.defaultAgent': 'kimi', 'acpira.appearance.font': 'inter', 'acpira.sessionScope': 'workspace', 'acpira.sessionListPosition': 'right', 'acpira.agents': { codex: { command: 'C:\\codex.cmd' } }, 'acpira.': 1, 'editor.tabSize': 2 }))
      .toEqual({ defaultAgent: 'kimi', 'appearance.font': 'inter' });
    expect(acpiraKeys([])).toEqual({});
    expect(acpiraKeys(null)).toEqual({});
  });

  it('looks where each OS keeps the user settings, VS Code before Cursor', () => {
    expect(ideSettingsPaths('darwin', {}, '/Users/u')).toEqual([
      join('/Users/u', 'Library', 'Application Support', 'Code', 'User', 'settings.json'),
      join('/Users/u', 'Library', 'Application Support', 'Cursor', 'User', 'settings.json'),
    ]);
    expect(ideSettingsPaths('linux', { XDG_CONFIG_HOME: '/cfg' }, '/home/u')[0]).toBe(join('/cfg', 'Code', 'User', 'settings.json'));
    expect(ideSettingsPaths('linux', {}, '/home/u')[0]).toBe(join('/home/u', '.config', 'Code', 'User', 'settings.json'));
    expect(ideSettingsPaths('win32', { APPDATA: 'C:\\AppData' }, 'C:\\Users\\u')[0]).toBe(join('C:\\AppData', 'Code', 'User', 'settings.json'));
  });
});
