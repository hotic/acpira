import { describe, expect, it } from 'vitest';
import { CODE_FONT_SIZE, DEFAULT_SETTINGS, inWorkspace, sanitizeSetting, UI_FONT_SIZE } from '../src/shared/settings';

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
