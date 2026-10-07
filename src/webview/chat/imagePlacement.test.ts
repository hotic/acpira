import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AgentTurn, ToolCallBlock } from '../../shared/transcript';
import { AppearanceContext, BASE_APPEARANCE } from '../appearance';
import { AgentMessage } from './Turns';
import { BlobUrlContext } from './fileLinks';
import { rememberFold, resetFoldMemory } from './foldMemory';

// The webview stores run only in the browser. Read their current snapshot during this DOM-free render check.
vi.mock('react', async importOriginal => {
  const react = await importOriginal<typeof import('react')>();
  return { ...react, useSyncExternalStore: <T>(subscribe: (changed: () => void) => () => void, snapshot: () => T, serverSnapshot?: () => T) =>
    react.useSyncExternalStore(subscribe, snapshot, serverSnapshot ?? snapshot) };
});

const before: ToolCallBlock = { type: 'tool_call', id: 'before', kind: 'read', verb: 'Read', target: 'before.png', status: 'completed' };
const after: ToolCallBlock = { type: 'tool_call', id: 'after', kind: 'execute', verb: 'Run', target: 'after-image-command', status: 'in_progress' };
const show: ToolCallBlock = { type: 'tool_call', id: 'show', kind: 'other', verb: 'Show image', verbKey: 'verb.showImage', status: 'completed',
  content: { type: 'image', blob: 'shown.png', mimeType: 'image/png' } };
const generation: ToolCallBlock = { ...show, id: 'generation', verb: 'Generate image', verbKey: 'verb.imagegen',
  content: { type: 'image', blob: 'generated.png', mimeType: 'image/png' } };

function render(blocks: AgentTurn['blocks'], running = true, fold: 'codex' | 'cursor' = 'codex', memoryKey?: string) {
  return renderToStaticMarkup(createElement(AppearanceContext.Provider, { value: { ...BASE_APPEARANCE, fold, motion: 'none' } },
    createElement(BlobUrlContext.Provider, { value: blob => `/test-images/${blob}` },
      createElement(AgentMessage, { turn: { role: 'agent', blocks }, running, index: 0, turnIndex: 1, last: true,
        memoryKey, actions: false, lead: 'static', onPermission: () => {} }))));
}

describe('image results in the shared process fold', () => {
  beforeEach(resetFoldMemory);

  it.each([show, generation])('keeps $verb at its tool position when later actions arrive', image => {
    const markup = render([before, image, after]);
    const imageAt = markup.indexOf('<img ');
    expect(imageAt).toBeGreaterThan(markup.indexOf('before.png'));
    expect(imageAt).toBeLessThan(markup.indexOf('after-image-command'));
    expect(markup.match(/<img /g)).toHaveLength(1);
  });

  it('preserves the order of multiple image results around later tools', () => {
    const markup = render([show, after, generation]);
    expect(markup.indexOf('src="/test-images/shown.png"')).toBeLessThan(markup.indexOf('after-image-command'));
    expect(markup.indexOf('src="/test-images/generated.png"')).toBeGreaterThan(markup.indexOf('after-image-command'));
    expect(markup.match(/<img /g)).toHaveLength(2);
  });

  it('keeps results visible when a finished process fold is closed', () => {
    const markup = render([show, { ...after, status: 'completed' }, generation], false);
    expect(markup).not.toContain('after-image-command');
    expect(markup.match(/<img /g)).toHaveLength(2);
  });

  it('shows each image once when a live process fold is closed by hand', () => {
    rememberFold('manually-closed-images', false);
    const markup = render([show, after, generation], true, 'codex', 'manually-closed-images');
    // Live panels keep their process DOM mounted; their image results must not duplicate the lifted copies.
    expect(markup).toContain('after-image-command');
    expect(markup.match(/<img /g)).toHaveLength(2);
  });

  it('retains chronological images in cursor mode', () => {
    const markup = render([show, after], true, 'cursor');
    expect(markup.indexOf('<img ')).toBeLessThan(markup.indexOf('after-image-command'));
    expect(markup.match(/<img /g)).toHaveLength(1);
  });
});
