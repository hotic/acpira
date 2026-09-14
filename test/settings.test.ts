import { describe, expect, it } from 'vitest';
import { CODE_FONT_SIZE, DEFAULT_SETTINGS, inWorkspace, sanitizeSetting, UI_FONT_SIZE } from '../src/shared/settings';
import { SettingsCenter, type SettingsDeps } from '../src/host/settings';

// settings.json is hand-editable and the setSetting message can come from any webview script: every appearance value is checked before it is used
describe('sanitizeSetting (appearance)', () => {
  it('theme / diffMarkers accept only their enums', () => {
    expect(sanitizeSetting('theme', 'light')).toBe('light');
    expect(sanitizeSetting('theme', 'dark')).toBe('dark');
    expect(sanitizeSetting('theme', 'system')).toBe('auto');
    expect(sanitizeSetting('theme', 3)).toBe('auto');
    expect(sanitizeSetting('diffMarkers', 'signs')).toBe('signs');
    expect(sanitizeSetting('diffMarkers', 'plusminus')).toBe('color');
  });

  it('font sizes are whole pixels clamped to their bounds, defaults otherwise', () => {
    expect(sanitizeSetting('uiFontSize', 14.4)).toBe(14);
    expect(sanitizeSetting('uiFontSize', 2)).toBe(UI_FONT_SIZE.min);
    expect(sanitizeSetting('uiFontSize', 99)).toBe(UI_FONT_SIZE.max);
    expect(sanitizeSetting('uiFontSize', '14')).toBe(UI_FONT_SIZE.default);
    expect(sanitizeSetting('codeFontSize', NaN)).toBe(CODE_FONT_SIZE.default);
    expect(sanitizeSetting('codeFontSize', 0)).toBe(CODE_FONT_SIZE.min);
  });

  it('fontSmoothing is a boolean', () => {
    expect(sanitizeSetting('fontSmoothing', true)).toBe(true);
    expect(sanitizeSetting('fontSmoothing', 'yes')).toBe(DEFAULT_SETTINGS.fontSmoothing);
  });

  it('sessionScope accepts workspace / all and defaults to workspace; a session is in a workspace when its cwd is that folder', () => {
    expect(sanitizeSetting('sessionScope', 'all')).toBe('all');
    expect(sanitizeSetting('sessionScope', 'workspace')).toBe('workspace');
    expect(sanitizeSetting('sessionScope', 'project')).toBe('workspace');
    expect(sanitizeSetting('sessionScope', undefined)).toBe('workspace');
    expect(inWorkspace({ cwd: '/w/a' }, '/w/a')).toBe(true);
    expect(inWorkspace({ cwd: '/w/a/sub' }, '/w/a')).toBe(false);
    expect(inWorkspace({ cwd: '/w/b' }, '/w/a')).toBe(false);
  });
});

function center(overrides: Partial<SettingsDeps> = {}) {
  const written: [string, unknown][] = [];
  const deps: SettingsDeps = {
    read: () => undefined,
    write: async (key, value) => { written.push([key, value]); },
    writeAppearance: async (axis, value) => { written.push([`appearance.${axis}`, value]); },
    hostLanguage: () => 'en',
    registry: () => { throw new Error('unused'); },
    runtimeInfo: () => undefined,
    home: () => '/home',
    cwd: () => '/cwd',
    ...overrides,
  };
  return { center: new SettingsCenter(deps), written };
}

describe('SettingsCenter', () => {
  it('round-trips session list placement through host settings and rejects unknown positions', async () => {
    const store: Record<string, unknown> = {};
    const { center: c } = center({ read: key => store[key], write: async (key, value) => { store[key] = value; } });
    const positions: string[] = [];
    c.subscribe(ev => positions.push(ev.settings.sessionListPosition));
    for (const value of ['left', 'right', 'hidden', 'outside']) await c.set('sessionListPosition', value);
    expect(positions).toEqual(['left', 'right', 'hidden', 'hidden']);
    await c.set('sessionListPosition', 'right');
    expect(center({ read: key => store[key] }).center.view().sessionListPosition).toBe('right');
  });

  it('view carries the appearance settings with defaults when nothing is configured', () => {
    const { center: c } = center();
    const v = c.view();
    expect(v.theme).toBe('auto');
    expect(v.uiFontSize).toBe(UI_FONT_SIZE.default);
    expect(v.codeFontSize).toBe(CODE_FONT_SIZE.default);
    expect(v.diffMarkers).toBe('color');
    expect(v.fontSmoothing).toBe(false);
  });

  it('setAppearance writes only values the axis declares', async () => {
    const { center: c, written } = center();
    await c.setAppearance('motion', 'none');
    await c.setAppearance('motion', 'hyper');
    await c.setAppearance('nonsense' as never, 'none');
    expect(written).toEqual([['appearance.motion', 'none']]);
  });
});
