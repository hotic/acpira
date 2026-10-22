import { describe, expect, it } from 'vitest';
import pkg from '../package.json';
import { AXES, BASE_APPEARANCE } from '../src/shared/appearance';

// VS Code reads the package.json defaults; every other host (LAB, harness, desktop, IDEA) falls back to the baseline
// combo code. Both must agree, or those hosts render a different transcript than the extension does
describe('appearance baseline', () => {
  it('matches the acpira.appearance.* defaults in package.json', () => {
    const sections = ([] as { properties?: Record<string, { default?: unknown }> }[]).concat(pkg.contributes.configuration as never);
    const props = Object.assign({}, ...sections.map(s => s.properties ?? {})) as Record<string, { default?: unknown }>;
    for (const ax of AXES) expect([ax.key, props[`acpira.appearance.${ax.key}`]?.default]).toEqual([ax.key, BASE_APPEARANCE[ax.key]]);
  });
});
