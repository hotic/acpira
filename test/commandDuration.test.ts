import { afterEach, describe, expect, it } from 'vitest';
import { setLocale } from '../src/webview/i18n';
import { commandDuration } from '../src/webview/chat/commandDuration';

afterEach(() => setLocale('en'));

describe('command run time', () => {
  it('keeps tenths below ten seconds and whole seconds beyond', () => {
    expect(commandDuration(180)).toBe('0.2s');
    expect(commandDuration(40)).toBe('<0.1s');
    expect(commandDuration(2_400)).toBe('2.4s');
    expect(commandDuration(41_800)).toBe('42s');
    expect(commandDuration(125_000)).toBe('2m 5s');
    expect(commandDuration(120_000)).toBe('2m');
    expect(commandDuration(-5)).toBe('<0.1s');
  });

  it('ticks a live clock in whole seconds', () => {
    expect(commandDuration(5_900, true)).toBe('5s');
    expect(commandDuration(0, true)).toBe('0s');
    expect(commandDuration(61_500, true)).toBe('1m 1s');
  });

  it('follows the locale', () => {
    setLocale('zh-CN');
    expect(commandDuration(180)).toBe('0.2 秒');
    expect(commandDuration(125_000)).toBe('2 分钟 5 秒');
  });
});
