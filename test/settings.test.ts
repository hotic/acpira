import { describe, expect, it } from 'vitest';
import { AGENT_CPU_CAP, CODE_FONT_SIZE, DEFAULT_SETTINGS, inWorkspace, sanitizeSetting, UI_FONT_SIZE } from '../src/shared/settings';

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

  it('the agent CPU cap is a whole percentage between 10 and 100, 80 otherwise', () => {
    expect(sanitizeSetting('agentCpuCap', 60)).toBe(60);
    expect(sanitizeSetting('agentCpuCap', 72.6)).toBe(73);
    expect(sanitizeSetting('agentCpuCap', 0)).toBe(AGENT_CPU_CAP.min);
    expect(sanitizeSetting('agentCpuCap', 250)).toBe(AGENT_CPU_CAP.max);
    expect(sanitizeSetting('agentCpuCap', '50')).toBe(80);
    expect(DEFAULT_SETTINGS.agentCpuCap).toBe(80);
  });

  // Mirrors acpira_shared::settings::proxy_setting: the engine and the page must agree on what a value means
  it('the proxy is auto, off or a normalized proxy URL; anything else reads as auto', () => {
    expect(DEFAULT_SETTINGS.proxy).toBe('auto');
    expect(sanitizeSetting('proxy', undefined)).toBe('auto');
    expect(sanitizeSetting('proxy', ' AUTO ')).toBe('auto');
    expect(sanitizeSetting('proxy', 'Direct')).toBe('off');
    expect(sanitizeSetting('proxy', 'http://127.0.0.1:7897/')).toBe('http://127.0.0.1:7897');
    expect(sanitizeSetting('proxy', '127.0.0.1:7890')).toBe('http://127.0.0.1:7890');
    expect(sanitizeSetting('proxy', 'socks5h://u:p@proxy:1080')).toBe('socks5h://u:p@proxy:1080');
    for (const bad of ['ftp://x:21', 'http://:8080', 'http://host:1/path', 'not a proxy']) expect(sanitizeSetting('proxy', bad)).toBe('auto');
    expect(sanitizeSetting('proxy', 7890)).toBe('auto');
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
