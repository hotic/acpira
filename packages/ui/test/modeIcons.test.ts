import { afterEach, describe, expect, it } from 'vitest';
import { LOCALES, translate } from '@acpira/shared/i18n';
import { AskShieldGlyph, AutoEditShieldGlyph, AutoShieldGlyph, CodeGlyph, FullAccessShieldGlyph, SmartGlyph } from '../chat/modeGlyphs';
import { accessIcon, modeIcon, modeLabel } from '../chat/modeIcons';
import { setLocale } from '../i18n';

afterEach(() => setLocale('en'));

describe('ACP mode presentation', () => {
  it('localizes known mode IDs in every shipped locale without changing the wire option', () => {
    const mode = Object.freeze({ id: 'code', name: 'Code', description: 'Write and edit code' });
    for (const locale of LOCALES) {
      setLocale(locale);
      expect(modeLabel(mode)).toBe(translate(locale, 'composer.modeShort.code'));
    }
    expect(mode).toEqual({ id: 'code', name: 'Code', description: 'Write and edit code' });
  });

  it('localizes Devin Code and Antigravity auto-edit using their actual wire IDs', () => {
    const devin = Object.freeze({ id: 'accept-edits', name: 'Code', description: 'Write and edit code' });
    const antigravity = Object.freeze({ id: 'auto_edit', name: 'Auto Edit' });
    for (const locale of LOCALES) {
      setLocale(locale);
      expect(modeLabel(devin)).toBe(translate(locale, 'composer.modeShort.code'));
      expect(modeLabel(antigravity)).toBe(translate(locale, 'composer.modeShort.autoEdit'));
      expect(modeLabel({ id: 'acceptEdits', name: 'Accept Edits' })).toBe(translate(locale, 'composer.modeShort.autoEdit'));
    }
    expect(devin.id).toBe('accept-edits');
    expect(antigravity.id).toBe('auto_edit');
  });

  it('distinguishes Codex auto review from the built-in Agent mode sharing its ID', () => {
    const codex = { id: 'agent', name: 'Auto review', kind: 'auto_review' };
    for (const locale of LOCALES) {
      setLocale(locale);
      expect(modeLabel(codex)).toBe(translate(locale, 'composer.modeShort.auto'));
      expect(modeLabel({ id: 'agent', name: 'Agent' })).toBe(translate(locale, 'composer.modeShort.agent'));
    }
    expect(modeIcon(codex)).toBe(SmartGlyph);
    expect(modeIcon({ id: 'custom', name: 'Custom', kind: 'auto_review' })).toBe(SmartGlyph);
  });

  it('uses the edit glyph for automatic file edits rather than automatic permission review', () => {
    expect(modeIcon({ id: 'auto_edit', name: 'Auto Edit' })).toBe(CodeGlyph);
    expect(modeIcon({ id: 'acceptEdits', name: 'Accept edits' })).toBe(CodeGlyph);
    expect(modeIcon({ id: 'auto', name: 'Auto' })).toBe(SmartGlyph);
  });

  it('maps the approval ladder to the solid shield family', () => {
    expect(accessIcon({ id: 'ask', name: 'Ask' })).toBe(AskShieldGlyph);
    expect(accessIcon({ id: 'auto-edit', name: 'Auto-edit' })).toBe(AutoEditShieldGlyph);
    expect(accessIcon({ id: 'auto', name: 'Auto' })).toBe(AutoShieldGlyph);
    expect(accessIcon({ id: 'full', name: 'Full access' })).toBe(FullAccessShieldGlyph);
  });

  it('keeps unknown and custom modes verbatim instead of guessing from their names', () => {
    setLocale('zh-CN');
    expect(modeLabel({ id: 'custom-review', name: 'Code Review' })).toBe('Code Review');
    expect(modeLabel({ id: 'toString', name: 'Custom' })).toBe('Custom');
  });

  it('gives the adapter full-access classification priority over an ordinary mode ID or name', () => {
    setLocale('zh-CN');
    const mode = { id: 'code', name: 'Code', kind: 'full_access' };
    expect(modeLabel(mode)).toBe('完全访问');
    expect(modeIcon(mode)).toBe(FullAccessShieldGlyph);
    expect(modeIcon({ id: 'custom', name: 'Custom', kind: 'full_access' })).toBe(FullAccessShieldGlyph);
  });
});
