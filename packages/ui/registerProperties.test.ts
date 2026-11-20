import { describe, expect, it, vi } from 'vitest';
import { CSS_PROPERTIES, registerCssProperties, type CssPropertyDef } from './registerProperties';

describe('registerCssProperties', () => {
  it('registers scrollbar, file-tree tint, beam, chat and tailwind gradient properties', () => {
    const calls: CssPropertyDef[] = [];
    registerCssProperties(def => calls.push(def));
    expect(calls).toEqual(CSS_PROPERTIES);
    const byName = new Map(calls.map(c => [c.name, c]));
    expect(byName.get('--scrollbar-thumb')).toEqual({
      name: '--scrollbar-thumb',
      syntax: '<color>',
      inherits: false,
      initialValue: 'transparent',
    });
    expect(byName.get('--scrollbar-inset')).toEqual({
      name: '--scrollbar-inset',
      syntax: '<length>',
      inherits: false,
      initialValue: '3.5px',
    });
    expect(byName.get('--tint')).toEqual({
      name: '--tint',
      syntax: '<color>',
      inherits: false,
      initialValue: 'transparent',
    });
    expect(byName.get('--bb-o16')).toEqual({
      name: '--bb-o16',
      syntax: '<number>',
      inherits: true,
      initialValue: '0',
    });
    expect(byName.get('--tw-gradient-from')?.syntax).toBe('<color>');
  });

  it('continues registering remaining properties when one throws', () => {
    const seen: string[] = [];
    registerCssProperties(def => {
      seen.push(def.name);
      if (def.name === '--scrollbar-thumb') throw new DOMException('already registered', 'InvalidModificationError');
    });
    expect(seen).toContain('--scrollbar-thumb');
    expect(seen).toContain('--scrollbar-inset');
    expect(seen).toContain('--tint');
    expect(seen.length).toBe(CSS_PROPERTIES.length);
  });

  it('no-ops cleanly when CSS.registerProperty is unavailable', () => {
    const orig = globalThis.CSS;
    try {
      (globalThis as { CSS?: unknown }).CSS = undefined;
      expect(() => registerCssProperties()).not.toThrow();
    } finally {
      (globalThis as { CSS?: unknown }).CSS = orig;
    }
  });

  it('registers against global CSS.registerProperty at most once', () => {
    const orig = globalThis.CSS;
    const registerProperty = vi.fn();
    try {
      (globalThis as { CSS?: unknown }).CSS = { registerProperty };
      registerCssProperties();
      registerCssProperties();
      expect(registerProperty).toHaveBeenCalledTimes(CSS_PROPERTIES.length);
    } finally {
      (globalThis as { CSS?: unknown }).CSS = orig;
    }
  });
});
