import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { DESKTOP_DEFAULTS, SettingsStore } from '../src/settings';

describe('SettingsStore', () => {
  const dirs: string[] = [];
  afterEach(() => { for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true }); });
  const file = () => { const d = mkdtempSync(join(tmpdir(), 'acpira-settings-')); dirs.push(d); return join(d, 'nested', 'settings.json'); };

  it('starts a first launch on the desktop defaults and persists them', () => {
    const f = file();
    const s = new SettingsStore(f);
    expect(s.fresh).toBe(true);
    expect(s.snapshot()).toEqual(DESKTOP_DEFAULTS);
    expect(JSON.parse(readFileSync(f, 'utf8'))).toEqual(DESKTOP_DEFAULTS);
    expect(new SettingsStore(f).fresh).toBe(false);
  });

  it('writes, removes and merges keys, announcing what changed', () => {
    const f = file();
    const s = new SettingsStore(f);
    const seen: string[][] = [];
    s.onChange(keys => seen.push(keys));
    s.set('defaultAgent', 'claude');
    s.set('sessionScope', undefined);
    s.merge({ 'appearance.font': 'inter', hiddenOptions: {} });
    s.merge({});
    expect(seen).toEqual([['defaultAgent'], ['sessionScope'], ['appearance.font', 'hiddenOptions']]);
    expect(new SettingsStore(f).snapshot()).toEqual({ sessionListPosition: 'left', defaultAgent: 'claude', 'appearance.font': 'inter', hiddenOptions: {} });
  });

  it('keeps a broken file aside instead of overwriting it with defaults', () => {
    const f = file();
    new SettingsStore(f);
    writeFileSync(f, '{ not json');
    const s = new SettingsStore(f);
    expect(s.fresh).toBe(false);
    expect(s.snapshot()).toEqual({});
    expect(readFileSync(`${f}.broken`, 'utf8')).toBe('{ not json');
  });
});
