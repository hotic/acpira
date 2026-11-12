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
