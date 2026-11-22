import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, expect, it, vi } from 'vitest';
import type { SubagentSummary } from '@acpira/shared/subagents';
import { SubagentInspector } from './SubagentInspector';
import { CommandStopContext } from '../fileLinks';
import { t } from '../../i18n';

// These browser-only stores have no server snapshot; keep their default state for the static render.
vi.mock('../useBottomFollow', async importOriginal => ({ ...await importOriginal<typeof import('../useBottomFollow')>(), useFollowing: () => true }));
vi.mock('../autoFold', async importOriginal => ({ ...await importOriginal<typeof import('../autoFold')>(), useAutoFold: () => undefined }));

// Existing output belongs to the observed snapshot, not to the live events that arrive after it.
afterEach(() => vi.unstubAllGlobals());
it('opens a running child snapshot without row entrances or replaying its streaming reply', () => {
  vi.stubGlobal('document', { activeElement: null });
  const node: SubagentSummary = {
    id: 'snapshot-quiet', turnIndex: 1, visibility: 'session', state: 'running', stateSource: 'agent',
    announcedAt: 1000, controls: { cancel: false }, peer: {}, toolCount: 2,
  };
  const html = renderToStaticMarkup(createElement(SubagentInspector, {
    node, mode: 'overlay', onClose: () => {}, onPermission: () => {},
    transcript: { rev: 10, running: true, turns: [{ role: 'agent', startedAt: 1000, blocks: [
      { type: 'tool_call', id: 'old-read', kind: 'read', verb: 'Read', status: 'completed', locations: [{ path: 'old.rs' }] },
      { type: 'text', markdown: 'Already written before opening the child.', streaming: true },
    ] }] },
  }));
  expect(html).toContain('old.rs');
  expect(html).toContain('Already written before opening the child.');
  expect(html).not.toContain('process-row-enter');
});

it('uses the child command stop context instead of the parent session context', () => {
  vi.stubGlobal('document', { activeElement: null });
  const node: SubagentSummary = {
    id: 'child', turnIndex: 1, visibility: 'session', state: 'running', stateSource: 'agent',
    announcedAt: 1000, controls: { cancel: false }, peer: {}, toolCount: 1,
  };
  const render = (stopCommand: ((id: string) => void) | undefined) => renderToStaticMarkup(
    createElement(CommandStopContext.Provider, { value: () => {} }, createElement(SubagentInspector, {
      node, mode: 'overlay', onClose: () => {}, onPermission: () => {}, stopCommand,
      transcript: { rev: 1, running: true, turns: [{ role: 'agent', blocks: [
        { type: 'tool_call', id: 'call-1', kind: 'execute', verb: 'Run', status: 'in_progress', target: 'cargo test', targetMono: true },
      ] }] },
    })),
  );
  expect(render(() => {})).toContain(`aria-label="${t('command.stop')}"`);
  expect(render(undefined)).not.toContain(`aria-label="${t('command.stop')}"`);
});

it('exposes the shared thread spacing hooks for both task cards and user turns', () => {
  vi.stubGlobal('document', { activeElement: null });
  const taskNode: SubagentSummary = {
    id: 'task-child', turnIndex: 1, visibility: 'session', state: 'running', stateSource: 'agent',
    announcedAt: 1000, controls: { cancel: false }, peer: {}, task: 'Audit release pipeline', toolCount: 1,
  };
  const taskHtml = renderToStaticMarkup(createElement(SubagentInspector, {
    node: taskNode, mode: 'docked', onClose: () => {}, onPermission: () => {},
    transcript: { rev: 1, running: true, turns: [{ role: 'agent', startedAt: 1000, blocks: [
      { type: 'tool_call', id: 'c1', kind: 'execute', verb: 'Run', status: 'in_progress', target: 'git status', targetMono: true },
    ] }] },
  }));
  expect(taskHtml).toContain('data-subagent-thread');
  // TaskCard and AgentMessage must be direct siblings so the prompt + process-head spacing selector matches
  expect(taskHtml).toMatch(/data-part="user-message"[\s\S]*<\/section><div data-part="agent-message"/);

  const threadedNode: SubagentSummary = {
    ...taskNode, id: 'threaded-child', harness: { agent: 'codex', persona: 'GPT', mode: 'consult', thread: 't1', round: 1 },
  };
  const threadedHtml = renderToStaticMarkup(createElement(SubagentInspector, {
    node: threadedNode, mode: 'docked', onClose: () => {}, onPermission: () => {},
    transcript: { rev: 2, running: true, turns: [
      { role: 'user', text: 'Inspect crates' },
      { role: 'user', text: 'auto prompt', auto: true },
      { role: 'agent', startedAt: 1000, blocks: [{ type: 'text', markdown: 'On it.' }] },
    ] },
  }));
  // Auto user turns omit their wrapper so no empty gap-msg sibling is inserted between the prompt and reply
  expect(threadedHtml.match(/data-part="user-message"/g)).toHaveLength(1);
  expect(threadedHtml).toMatch(/data-part="user-message" data-turn="0"[\s\S]*<\/div><div data-part="agent-message" data-turn="2"/);
});
