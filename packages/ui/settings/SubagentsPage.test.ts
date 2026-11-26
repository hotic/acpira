import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import { DEFAULT_SETTINGS } from '@acpira/shared/settings';
import type { SubagentGroup, SubagentPersona } from '@acpira/shared/subagents';
import type { AgentInfo } from '@acpira/shared/transcript';
import { SubagentsPage } from './SubagentsPage';
import type { SettingsPage } from './Nav';
import type { SettingsHandlers } from './SettingsShell';
import { moveById, orderById } from './SortableRows';

const agents: AgentInfo[] = [
  { id: 'claude', name: 'Claude Code', available: true },
  { id: 'codex', name: 'Codex', available: true },
];

const persona = (id: string, name: string): SubagentPersona => ({
  id, name, agent: 'claude', mode: 'bypass', when: '', enabled: true,
});

const group = (id: string, name: string, members: string[]): SubagentGroup => ({
  id, name, members, when: '', enabled: true,
});

const handlers: SettingsHandlers = {
  setSetting: vi.fn(),
  saveSubagents: vi.fn(),
  saveSubagentGroups: vi.fn(),
  setAppearance: vi.fn(),
  openPath: vi.fn(),
  refreshInventory: vi.fn(),
  refreshAgent: vi.fn(),
  selectAccount: vi.fn(),
  addAccount: vi.fn(),
  removeAccount: vi.fn(),
  installAgent: vi.fn(),
  openExternal: vi.fn(),
};

function renderPage(subagents: SubagentPersona[], subagentGroups: SubagentGroup[] = [], page: Extract<SettingsPage, { kind: 'subagents' }> = { kind: 'subagents' }) {
  return renderToStaticMarkup(createElement(SubagentsPage, {
    page,
    onPage: vi.fn(),
    onReplace: vi.fn(),
    settings: { ...DEFAULT_SETTINGS, subagents, subagentGroups },
    agents,
    controls: {},
    on: handlers,
  }));
}

describe('SubagentsPage reorder', () => {
  it('exposes keyboard reorder shortcuts only when there is more than one persona', () => {
    const single = renderPage([persona('opus', 'Opus')]);
    expect(single).toContain('data-persona-row="opus"');
    expect(single).not.toContain('Alt+ArrowUp Alt+ArrowDown');

    const multi = renderPage([persona('opus', 'Opus'), persona('astra', 'Astra')]);
    expect(multi).toContain('data-persona-row="opus"');
    expect(multi).toContain('data-persona-row="astra"');
    expect(multi).toContain('aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown"');
  });

  it('disables row dragging while a persona dialog is open', () => {
    const open = renderPage([persona('opus', 'Opus'), persona('astra', 'Astra')], [], { kind: 'subagents', persona: 'opus' });
    expect(open).not.toContain('Alt+ArrowUp Alt+ArrowDown');
  });

  it('exposes group reorder rows and shortcuts only when there is more than one group', () => {
    const personas = [persona('opus', 'Opus'), persona('astra', 'Astra')];
    const single = renderPage([persona('opus', 'Opus')], [group('review', 'Review', ['opus'])]);
    expect(single).toContain('data-group-row="review"');
    expect(single).not.toContain('Alt+ArrowUp Alt+ArrowDown');

    const multi = renderPage(personas, [group('review', 'Review', ['opus']), group('build', 'Build', ['astra'])]);
    expect(multi).toContain('data-group-row="review"');
    expect(multi).toContain('data-group-row="build"');
    expect(multi).toContain('aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown"');
  });

  it('moves and orders items by id cleanly', () => {
    const list = [persona('a', 'A'), persona('b', 'B'), persona('c', 'C')];
    expect(moveById(list, 'a', 2).map(p => p.id)).toEqual(['b', 'c', 'a']);
    expect(moveById(list, 'c', 0).map(p => p.id)).toEqual(['c', 'a', 'b']);
    expect(moveById(list, 'a', 0)).toBe(list);
    expect(orderById(list, ['c', 'missing', 'a']).map(p => p.id)).toEqual(['c', 'a', 'b']);
  });
});
