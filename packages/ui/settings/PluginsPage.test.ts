import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AgentInfo } from '@acpira/shared/transcript';
import type { AgentInventory } from '@acpira/shared/inventory';
import { setLocale, t } from '../i18n';
import { ModelsPage } from './PluginsPage';

const inv = (agent: string, extra: Partial<AgentInventory> = {}): AgentInventory => ({
  agent, binary: `/bin/${agent}`, steer: false, config: [], mcp: [], skills: [], rules: [], scannedAt: '2026-11-21T00:00:00Z', ...extra,
});

function renderTiles(agents: AgentInfo[], inventories: Partial<Record<string, AgentInventory>>) {
  return renderToStaticMarkup(createElement(ModelsPage, {
    agents, inventories, onReorder: vi.fn(), onDisabled: vi.fn(), onOpen: vi.fn(), onInventory: vi.fn(), onUpdate: vi.fn(),
  }));
}

afterEach(() => setLocale('en'));

describe('ModelsPage agent tiles', () => {
  it('shows Built in alone for the built-in agent when no version is reported, and Built in with the version when known', () => {
    setLocale('zh-CN');
    const builtin: AgentInfo = { id: 'acpira', name: 'Acpira', available: true };

    const probing = renderTiles([builtin], {});
    expect(probing).toContain(t('settings.plugins.bundled'));
    expect(probing).not.toContain(t('settings.agent.probing'));
    expect(probing).not.toContain(t('settings.fact.versionUnknown'));

    const unknown = renderTiles([builtin], { acpira: inv('acpira') });
    expect(unknown).toContain(t('settings.plugins.bundled'));
    expect(unknown).not.toContain(t('settings.fact.versionUnknown'));

    const known = renderTiles([builtin], { acpira: inv('acpira', { cliVersion: '2.1.6' }) });
    expect(known).toContain(t('settings.plugins.bundled'));
    expect(known).toContain('2.1.6');
    expect(known).not.toContain(t('settings.fact.versionUnknown'));
  });

  it('keeps probing, unknown-version and not-installed states for external CLI tiles', () => {
    const cli: AgentInfo = { id: 'grok', name: 'Grok Build', available: true };
    expect(renderTiles([cli], {})).toContain(t('settings.agent.probing'));
    expect(renderTiles([cli], { grok: inv('grok') })).toContain(t('settings.fact.versionUnknown'));
    expect(renderTiles([{ ...cli, available: false }], {})).toContain(t('settings.plugins.notInstalled'));
    expect(renderTiles([cli], { grok: inv('grok', { cliVersion: '1.0.77' }) })).toContain('1.0.77');
  });
});
