import { describe, expect, it } from 'vitest';
import pkg from '../apps/vscode/package.json';
import { isSharedSetting, SHELL_SETTING_KEYS } from '../packages/shared/settings';

// Shared settings (the appearance axes among them) live in <ACPIRA_HOME>/settings.json for every shell, so no host
// reads package.json defaults for them any more: every host falls back to the engine's defaults and the baseline combo
// code. A shared key declared in the manifest again would bring back a second, VS Code-only copy with its own default
describe('VS Code settings manifest', () => {
  it('declares only the shell\'s own settings and the legacy agent list', () => {
    const sections = ([] as { properties?: Record<string, unknown> }[]).concat(pkg.contributes.configuration as never);
    const keys = sections.flatMap(s => Object.keys(s.properties ?? {})).map(k => k.replace(/^acpira\./, ''));
    expect(keys.filter(isSharedSetting)).toEqual([]);
    expect(keys.sort()).toEqual([...SHELL_SETTING_KEYS, 'agents'].sort());
  });
});
