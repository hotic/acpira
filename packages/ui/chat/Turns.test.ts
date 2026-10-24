import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import type { SubagentSummary } from '@acpira/shared/subagents';
import type { AgentTurn } from '@acpira/shared/transcript';
import { setLocale } from '../i18n';
import { AgentMessage } from './Turns';

// The bottom-follow and per-row fold stores have no server snapshot: a reader at the bottom is what a live turn assumes,
// and rows without an automatic fold state fall back to their own
vi.mock('./useBottomFollow', () => ({ useFollowing: () => true }));
vi.mock('./autoFold', async importOriginal => ({ ...await importOriginal<typeof import('./autoFold')>(), useAutoFold: () => undefined }));

setLocale('en');

const node = (id: string, title: string, activity: string): SubagentSummary => ({
  id, turnIndex: 1, visibility: 'session', state: 'running', stateSource: 'agent', controls: { cancel: true }, announcedAt: 0,
  title, task: title, activity, toolCount: 0, peer: { sessionId: `relay:${id}` },
});

const render = (turn: AgentTurn, running: boolean, nodes: SubagentSummary[]) => renderToStaticMarkup(createElement(AgentMessage, {
  turn, index: 1, running, onPermission: () => {}, turnIndex: 1, last: true, subagents: nodes, allSubagents: nodes, onInspect: () => {},
}));

// Reported 2026-10-14 on the README's summon clip: the work before a summon closed into a Done fold while the turn still
// ran, and a second Working row sat under the subagent rows
describe('a turn that summons subagents', () => {
  const nodes = [node('n1', 'Review the diff', 'Reading src/Search.tsx'), node('n2', 'Test edge cases', 'Thinking')];
  const turn: AgentTurn = { role: 'agent', startedAt: 1, blocks: [
    { type: 'thought', text: 'Ask both in parallel.' },
    { type: 'tool_call', id: 'd1', kind: 'other', verb: 'ask_agent', target: 'mcp__acpira__ask_agent', status: 'in_progress', subagentId: 'n1' },
    { type: 'tool_call', id: 'd2', kind: 'other', verb: 'ask_agent', target: 'mcp__acpira__ask_agent', status: 'in_progress', subagentId: 'n2' },
  ] };

  it('keeps one Working head over the thought and the subagent rows while it runs', () => {
    const html = render(turn, true, nodes);
    // One fold head (its title is the label; the shimmer draws the text twice)
    expect(html.match(/title="Working"/g)).toHaveLength(1);
    expect(html).not.toContain('title="Done"');
    // Both rows sit in the process under that head; the delegation rows never render as tools
    expect(html.indexOf('title="Working"')).toBeLessThan(html.indexOf('Review the diff'));
    expect(html.indexOf('Review the diff')).toBeLessThan(html.indexOf('Test edge cases'));
    expect(html).not.toContain('mcp__acpira__ask_agent');
  });

  it('closes into one Done head with the rows still in view once the turn ends', () => {
    const done: AgentTurn = { ...turn, endedAt: 9_001, stop: 'end_turn', blocks: [...turn.blocks, { type: 'text', markdown: 'Both agree.' }] };
    const html = render(done, false, nodes.map(n => ({ ...n, state: 'completed', activity: undefined, result: `${n.title}: fine.` })));
    expect(html.match(/title="Done"/g)).toHaveLength(1);
    expect(html).not.toContain('title="Working"');
    expect(html.indexOf('title="Done"')).toBeLessThan(html.indexOf('Review the diff'));
    expect(html.indexOf('Test edge cases')).toBeLessThan(html.indexOf('Both agree.'));
  });
});
