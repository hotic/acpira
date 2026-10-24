import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { cn } from '../packages/ui/ui/cn';

// Spacing keys the @theme block maps to utilities; cn.ts must know each one or a later token class cannot replace an earlier one
const themeSpacing = [...readFileSync(join(import.meta.dirname, '../packages/ui/styles/tokens.css'), 'utf8')
  .matchAll(/--spacing-([a-z0-9-]+):/g)].map(m => m[1]);

describe('token-aware class merging', () => {
  it('keeps independent typography and color tokens, including opacity and state modifiers', () => {
    expect(cn('text-2 text-fg-2')).toBe('text-2 text-fg-2');
    expect(cn('text-mono text-fg-1/85')).toBe('text-mono text-fg-1/85');
    expect(cn('hover:text-2 hover:text-fg-1')).toBe('hover:text-2 hover:text-fg-1');
    expect(cn('text-2', 'text-3 text-fg-1')).toBe('text-3 text-fg-1');
  });
  it('replaces token geometry and shadows without swallowing unrelated properties', () => {
    expect(cn('size-ctl', 'size-ctl-sm')).toBe('size-ctl-sm');
    expect(cn('px-pad py-pad-y', 'px-page')).toBe('py-pad-y px-page');
    expect(cn('pt-1 pl-indent', 'pt-gap-half')).toBe('pl-indent pt-gap-half');
    expect(cn('shadow-card', 'shadow-none')).toBe('shadow-none');
    expect(cn('bg-card border-card-line', 'bg-bg-1')).toBe('border-card-line bg-bg-1');
  });
  it('knows every spacing token declared in tokens.css', () => {
    expect(themeSpacing.length).toBeGreaterThan(40);
    const unknown = themeSpacing.filter(key => cn('px-pad', `px-${key}`) !== `px-${key}`);
    expect(unknown).toEqual([]);
  });
});
