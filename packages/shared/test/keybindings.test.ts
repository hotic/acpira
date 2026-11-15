import { describe, expect, it } from 'vitest';
import {
  SHORTCUT_DEFS,
  effectiveShortcut,
  formatShortcut,
  matchesShortcut,
  normalizeAccelerator,
  recordShortcutEvent,
  sanitizeKeybindings,
  type KeyEventLike,
} from '../keybindings';
import { isSharedSetting, sanitizeSetting } from '../settings';

const key = (overrides: Partial<KeyEventLike>): KeyEventLike => ({
  key: '',
  code: '',
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...overrides,
});

describe('keybindings', () => {
  it('normalizes modifier order and rejects unsafe plain-typing keys', () => {
    expect(normalizeAccelerator('cmd + shift + m')).toBe('Shift+CmdOrCtrl+M');
    expect(normalizeAccelerator('Ctrl + CmdOrCtrl + b')).toBe('Ctrl+CmdOrCtrl+B');
    expect(normalizeAccelerator('ctrl+`')).toBe('Ctrl+`');
    expect(normalizeAccelerator('Shift+Tab')).toBe('Shift+Tab');
    expect(normalizeAccelerator('F3')).toBe('F3');
    // Bare typing keys and Shift+letter belong to text input
    expect(normalizeAccelerator('a')).toBeUndefined();
    expect(normalizeAccelerator('Shift+A')).toBeUndefined();
    expect(normalizeAccelerator('Space')).toBeUndefined();
    expect(normalizeAccelerator('Enter')).toBeUndefined();
  });

  it('sanitizes keybindings maps, keeping unbound empty strings and dropping defaults or unknown ids', () => {
    expect(isSharedSetting('keybindings')).toBe(true);
    const cleaned = sanitizeSetting('keybindings', {
      toggleBrowser: 'Ctrl+Cmd+B',
      openSettings: '',
      newSession: 'CmdOrCtrl+N', // matches default -> omitted
      unknownAction: 'CmdOrCtrl+K',
      focusComposer: 'plain-key',
    });
    expect(cleaned).toEqual({
      toggleBrowser: 'Ctrl+CmdOrCtrl+B',
      openSettings: '',
    });
    expect(effectiveShortcut('toggleBrowser', cleaned)).toBe('Ctrl+CmdOrCtrl+B');
    expect(effectiveShortcut('openSettings', cleaned)).toBe('');
    expect(effectiveShortcut('newSession', cleaned)).toBe('CmdOrCtrl+N');
  });

  it('matches default, secondary platform aliases, customized and unbound shortcuts', () => {
    // ⌘⇧] secondary alias for nextSession on macOS when uncustomized
    expect(matchesShortcut(key({ code: 'BracketRight', key: '}', metaKey: true, shiftKey: true }), 'nextSession', {}, true)).toBe(true);
    expect(matchesShortcut(key({ code: 'Tab', key: 'Tab', ctrlKey: true }), 'nextSession', {}, true)).toBe(true);
    // Customizing nextSession disables the secondary alias
    const custom = sanitizeKeybindings({ nextSession: 'Alt+CmdOrCtrl+Right' });
    expect(matchesShortcut(key({ code: 'BracketRight', key: '}', metaKey: true, shiftKey: true }), 'nextSession', custom, true)).toBe(false);
    expect(matchesShortcut(key({ code: 'ArrowRight', key: 'ArrowRight', metaKey: true, altKey: true }), 'nextSession', custom, true)).toBe(true);
    // Unbinding disables matching completely
    expect(matchesShortcut(key({ code: 'KeyF', key: 'f', metaKey: true }), 'findInChat', { findInChat: '' }, true)).toBe(false);
    // ⌘⌥A on macOS produces e.key = "å", matched reliably via physical e.code = "KeyA"
    expect(matchesShortcut(key({ code: 'KeyA', key: 'å', metaKey: true, altKey: true }), 'nextAttention', {}, true)).toBe(true);
    expect(matchesShortcut(key({ code: 'KeyA', key: 'a', ctrlKey: true, altKey: true }), 'nextAttention', {}, false)).toBe(true);
  });

  it('records chords, Esc cancel and Backspace clear, and formats platform glyphs', () => {
    expect(recordShortcutEvent(key({ key: 'Escape', code: 'Escape' }), true)).toBe('cancel');
    expect(recordShortcutEvent(key({ key: 'Backspace', code: 'Backspace' }), true)).toBe('clear');
    expect(recordShortcutEvent(key({ key: 'Meta', code: 'MetaLeft', metaKey: true }), true)).toBeUndefined();
    expect(recordShortcutEvent(key({ key: 'b', code: 'KeyB', ctrlKey: true, metaKey: true }), true)).toBe('Ctrl+CmdOrCtrl+B');
    expect(formatShortcut('Ctrl+CmdOrCtrl+B', true)).toBe('⌃⌘B');
    expect(formatShortcut('CmdOrCtrl+Shift+M', true)).toBe('⇧⌘M');
    expect(formatShortcut('CmdOrCtrl+Shift+M', false)).toBe('Ctrl+Shift+M');
    for (const def of SHORTCUT_DEFS) {
      expect(normalizeAccelerator(def.defaultKey), def.id).toBeTruthy();
    }
  });
});
