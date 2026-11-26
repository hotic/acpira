import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AgentInfo } from '@acpira/shared/transcript';
import type { AgentInventory } from '@acpira/shared/inventory';
import type { PluginInfo, PluginsView } from '@acpira/shared/plugins';
import { setLocale, t } from '../i18n';
import { setPluginsView } from '../plugins/skin';
import { mergeTileOrder, ModelsPage, PluginsPage } from './PluginsPage';
import { resolvePluginOverlaps } from './UserPlugins';

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

const plugin = (id: string, name: string, contributes: PluginInfo['contributes']): PluginInfo => ({
  id, name, description: `${name} desc`, version: '1.0.0', layer: 'tools', contributes,
  enabled: true, loaded: true, canRollback: false, changed: false, path: `/plugins/${id}`,
});

describe('PluginsPage tile order', () => {
  it('merges a reordered subset back into the full list without moving unseen tiles', () => {
    const full = ['builtin:computer', 'asgard', 'keke-bridge', 'keke-phone', 'translator'];
    // Reorder inside the Tools filter (`builtin:computer`, `keke-bridge`, `keke-phone`)
    expect(mergeTileOrder(full, ['keke-phone', 'builtin:computer', 'keke-bridge'], ['other-os']))
      .toEqual(['keke-phone', 'asgard', 'builtin:computer', 'keke-bridge', 'translator', 'other-os']);
  });

  it('renders built-in computer use and user plugins in their saved display order', () => {
    const view: PluginsView = {
      root: '/plugins',
      generation: 1,
      safeMode: false,
      safeModeForced: false,
      plugins: [
        plugin('asgard', 'Asgard', ['ui']),
        plugin('keke-bridge', 'Keke Bridge', ['tools']),
      ],
      toolGroups: [{ id: 'computer', on: false, tools: [] }],
      changes: [],
      order: ['keke-bridge', 'asgard', 'builtin:computer'],
    };
    setPluginsView(view);
    const html = renderToStaticMarkup(createElement(PluginsPage, { tab: 'all', onTab: vi.fn() }));
    const ids = [...html.matchAll(/data-sortable-tile="([^"]+)"/g)].map(m => m[1]);
    expect(ids).toEqual(['keke-bridge', 'asgard', 'builtin:computer']);
  });

  it('splits overlapping peers into loads-before and loads-after lists based on tile order', () => {
    setLocale('zh-CN');
    const a: PluginInfo = {
      ...plugin('alpha', 'Alpha', ['style', 'commands']),
      overlaps: [
        { plugin: 'beta', kind: 'command', items: ['/review'] },
        { plugin: 'beta', kind: 'style', items: ['[data-session]'] },
        { plugin: 'gamma', kind: 'hook', items: ['prompt'] },
      ],
    };
    const names = new Map([['alpha', 'Alpha'], ['beta', 'Beta'], ['gamma', 'Gamma']]);

    // When alpha is first, it loads before both beta and gamma (multiple kinds merged per peer)
    const leading = resolvePluginOverlaps(a, ['alpha', 'beta', 'gamma'], names);
    expect(leading.after).toEqual([]);
    expect(leading.before).toEqual([
      { id: 'beta', name: 'Beta', summary: '命令 /review、样式' },
      { id: 'gamma', name: 'Gamma', summary: '钩子 prompt' },
    ]);

    // Dragging beta ahead of alpha flips beta into the loads-after group
    const middle = resolvePluginOverlaps(a, ['beta', 'alpha', 'gamma'], names);
    expect(middle.after).toEqual([{ id: 'beta', name: 'Beta', summary: '命令 /review、样式' }]);
    expect(middle.before).toEqual([{ id: 'gamma', name: 'Gamma', summary: '钩子 prompt' }]);

    // Unloaded plugins never show conflict badges
    expect(resolvePluginOverlaps({ ...a, loaded: false }, ['alpha', 'beta', 'gamma'], names)).toEqual({ before: [], after: [] });
  });

  it('renders the overlap trigger only on loaded tiles with overlaps and warns when shadowed', () => {
    setLocale('zh-CN');
    const view: PluginsView = {
      root: '/plugins',
      generation: 2,
      safeMode: false,
      safeModeForced: false,
      plugins: [
        { ...plugin('alpha', 'Alpha', ['style']), overlaps: [{ plugin: 'beta', kind: 'style', items: ['[data-session]'] }] },
        { ...plugin('beta', 'Beta', ['style']), overlaps: [{ plugin: 'alpha', kind: 'style', items: ['[data-session]'] }] },
        plugin('gamma', 'Gamma', ['tools']),
      ],
      toolGroups: [],
      changes: [],
      order: ['alpha', 'beta', 'gamma'],
    };
    setPluginsView(view);
    const html = renderToStaticMarkup(createElement(PluginsPage, { tab: 'all', onTab: vi.fn() }));
    const triggers = [...html.matchAll(/data-plugin-overlap-trigger[^>]*class="([^"]+)"/g)].map(m => m[1]);
    expect(triggers).toHaveLength(2);
    // alpha leads (neutral text-fg-2), beta trails behind alpha (text-warn), gamma has no overlap
    expect(triggers[0]).toContain('text-fg-2');
    expect(triggers[0]).not.toContain('text-warn');
    expect(triggers[1]).toContain('text-warn');
  });
});
