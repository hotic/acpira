import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import type { AgentTurn, ToolCallBlock } from '../../shared/transcript';
import { AppearanceContext, BASE_APPEARANCE } from '../appearance';
import { AgentMessage } from './Turns';
import { TAIL_SIZE, TAIL_THRESHOLD } from './tailWindow';

// The webview stores run only in the browser. Read their current snapshot during this DOM-free render check.
vi.mock('react', async importOriginal => {
  const react = await importOriginal<typeof import('react')>();
  return { ...react, useSyncExternalStore: <T>(subscribe: (changed: () => void) => () => void, snapshot: () => T, serverSnapshot?: () => T) =>
    react.useSyncExternalStore(subscribe, snapshot, serverSnapshot ?? snapshot) };
});

const commands = (n: number): ToolCallBlock[] => Array.from({ length: n }, (_, i) => ({
  type: 'tool_call', id: `call-${i}`, kind: 'execute', verb: 'Run', target: `command-${String(i).padStart(4, '0')}`, status: 'completed',
}));

function render(blocks: AgentTurn['blocks'], running: boolean, fold: 'codex' | 'cursor') {
  return renderToStaticMarkup(createElement(AppearanceContext.Provider, { value: { ...BASE_APPEARANCE, fold, motion: 'none' } },
    createElement(AgentMessage, { turn: { role: 'agent', blocks }, running, index: 0, turnIndex: 1, last: true,
      actions: false, lead: 'static', onPermission: () => {} })));
}

const target = (i: number) => `command-${String(i).padStart(4, '0')}`;

describe('tail window of a live turn', () => {
  it('first renders only the tail of a long live turn', () => {
    const n = TAIL_THRESHOLD + 90;
    const markup = render(commands(n), true, 'cursor');
    expect(markup).toContain(target(n - 1));
    expect(markup).toContain(target(n - TAIL_SIZE));
    expect(markup).not.toContain(target(n - TAIL_SIZE - 1));
    expect(markup).not.toContain(target(0));
  });

  // The process fold cuts its rows into chunks instead (ProcessChunks.tsx): where a page can tell what is near, a
  // working turn first has the rows at both ends and sized boxes between them
  it('first renders the ends of a long live process, the middle as empty boxes', () => {
    vi.stubGlobal('IntersectionObserver', class {});
    vi.stubGlobal('window', { innerHeight: 800 });
    try {
      const n = TAIL_THRESHOLD + 90;
      const markup = render(commands(n), true, 'codex');
      expect(markup).toContain(target(0));
      expect(markup).toContain(target(n - 1));
      expect(markup).not.toContain(target(n >> 1));
      expect(markup).toMatch(/data-chunk="[^"]+" data-calls="[^"]*call-\d+[^"]*" class="shrink-0" style="height:calc\(24 \* /);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it.each(['codex', 'cursor'] as const)('renders a live turn under the threshold whole (%s)', fold => {
    const markup = render(commands(TAIL_THRESHOLD), true, fold);
    expect(markup).toContain(target(0));
    expect(markup).toContain(target(TAIL_THRESHOLD - 1));
  });

  it('renders a settled turn without a window', () => {
    const n = TAIL_THRESHOLD + 90;
    const markup = render(commands(n), false, 'cursor');
    expect(markup).toContain(target(0));
    expect(markup).toContain(target(n - 1));
  });
});
