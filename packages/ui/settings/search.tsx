import type { ReactNode } from 'react';
import { Blocks, BrainCircuit, HardDrive, Info, Layers, Network, Palette, SlidersHorizontal, Workflow } from 'lucide-react';
import type { MsgKey } from '@acpira/shared/i18n/core';
import type { PluginKind } from '@acpira/shared/plugins';
import type { AgentInfo } from '@acpira/shared/transcript';
import { t } from '../i18n';
import type { SettingsPage } from './Nav';

// The rail's search, Codex-style: every page, every titled group and every field is an entry, and a hit that is not a
// page itself names the page it lives on. Names may repeat across pages (Appearance the page, Appearance the plugins
// filter); the second line tells them apart. Matched on the shown names only, never on descriptions

export interface SearchEntry {
  label: string;
  // The page it lives on, when the entry is not the page itself
  where?: string;
  icon: ReactNode;
  page: SettingsPage;
  // A group head or a field label on that page, brought into view after the jump
  anchor?: string;
}

export type SearchPage = 'general' | 'appearance' | 'models' | 'subagents' | 'plugins' | 'shared' | 'connections' | 'storage' | 'about';

const ICON: Record<SearchPage, ReactNode> = {
  general: <SlidersHorizontal />, appearance: <Palette />, models: <BrainCircuit />, subagents: <Workflow />, plugins: <Blocks />,
  shared: <Layers />, connections: <Network />, storage: <HardDrive />, about: <Info />,
};

// The plugins filters' names, plus the words someone looking for one may type
export const PLUGIN_KIND_LABEL: Record<PluginKind, MsgKey> = {
  look: 'settings.plugins.appearance', ui: 'settings.plugins.interface', models: 'settings.nav.models', tools: 'settings.plugins.tools', automation: 'settings.plugins.automation',
};
const PLUGIN_KIND_ALIASES: Record<PluginKind, MsgKey[]> = {
  look: ['settings.plugins.skins', 'settings.plugins.styles'], ui: [], models: ['settings.plugins.sources'],
  tools: ['settings.shared.tab.mcp'], automation: ['settings.plugins.hooks', 'settings.plugins.schedules'],
};

// What each page holds, in its own order: group heads and field labels as the page shows them. Keep in step with the
// pages (General.tsx, Appearance.tsx, …): an entry whose label is not on the page lands on the page's top
function holds(page: SearchPage, desktop: boolean): MsgKey[] {
  switch (page) {
    case 'general': return ['settings.group.basic', 'settings.language', 'settings.defaultAgent', desktop ? 'settings.openFilesWith' : 'settings.sessionScope',
      'settings.group.input', 'settings.shareEditorSelection', 'settings.steerQueued', 'settings.compaction.title', 'settings.autoCompact',
      'settings.compactAt', 'settings.group.run', 'settings.accountSwitch', 'settings.agentCpuCap', 'settings.proxy',
      'settings.notifications.title', 'settings.notifySound', 'settings.notifySystem'];
    case 'appearance': return ['settings.group.ui', desktop ? 'settings.desktopLayout' : 'settings.sessionListPosition', 'settings.theme', 'settings.motion',
      'settings.autoExpand', 'settings.reportLink', 'settings.diffMarkers', 'settings.panels', 'settings.plugins.files', 'settings.plugins.terminal', 'settings.plugins.browser',
      ...(desktop ? ['settings.panelScope' as const] : []), 'settings.text.title', 'settings.uiFontSize', 'settings.codeFontSize', 'settings.fontSmoothing'];
    case 'subagents': return ['subagents.page.list', 'subagents.groups.list'];
    case 'shared': return ['settings.shared.tab.skills', 'settings.shared.tab.mcp', 'settings.shared.tab.prompts'];
    case 'connections': return ['connections.hosts'];
    case 'storage': return ['storage.cleanup'];
    default: return [];
  }
}

export const pageName = (page: SearchPage) => t(`settings.nav.${page}`);

export function searchIndex(agents: AgentInfo[], pages: SearchPage[], desktop: boolean): SearchEntry[] {
  const out: SearchEntry[] = [];
  for (const page of pages) {
    const where = pageName(page);
    out.push({ label: where, icon: ICON[page], page: { kind: page } as SettingsPage });
    for (const key of holds(page, desktop)) {
      const label = t(key);
      out.push({ label, where, icon: ICON[page], page: { kind: page } as SettingsPage, anchor: label });
    }
    // Every agent opens its dialog on the Models page; every plugins filter opens on the Plugins page
    if (page === 'models') for (const a of agents) out.push({ label: a.name, where, icon: ICON.models, page: { kind: 'agent', id: a.id } });
    if (page === 'plugins') {
      for (const kind of Object.keys(PLUGIN_KIND_LABEL) as PluginKind[]) {
        for (const key of [PLUGIN_KIND_LABEL[kind], ...PLUGIN_KIND_ALIASES[kind]]) out.push({ label: t(key), where, icon: ICON.plugins, page: { kind: 'plugins', tab: kind } });
      }
    }
  }
  return out;
}

// Pages first, then exact names, then the rest in page order
export function searchHits(index: SearchEntry[], query: string): SearchEntry[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  const rank = (e: SearchEntry) => (e.where ? 2 : 0) + (e.label.toLowerCase() === q ? 0 : 1);
  return index.map((e, i) => ({ e, i })).filter(x => x.e.label.toLowerCase().includes(q))
    .sort((a, b) => rank(a.e) - rank(b.e) || a.i - b.i).map(x => x.e);
}

// After a jump: the group head, field label or tab with that text, brought into view once the page has rendered
export function scrollToAnchor(from: Element, anchor: string) {
  const root = from.closest('.acp-settings');
  const attempt = (left: number) => requestAnimationFrame(() => {
    const el = [...(root?.querySelectorAll('main label, main h3 span, main [role=tab]') ?? [])].find(x => x.textContent?.trim() === anchor);
    if (el) el.scrollIntoView({ block: 'center', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
    else if (left > 0) attempt(left - 1);
  });
  attempt(10);
}
