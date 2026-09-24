import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AgentTurn } from '../src/shared/transcript';
import { setLocale } from '../src/webview/i18n';
import { foldActivity } from '../src/webview/chat/folding';
import { agentTurn } from './fixtures/engine';

afterEach(() => { vi.restoreAllMocks(); setLocale('en'); });

describe('thought activity without a reasoning-end signal', () => {
  it('keeps an unreported generation gap generic, then switches to the reported tool', () => {
    vi.spyOn(Date, 'now').mockReturnValue(38000);
    expect(foldActivity(agentTurn('thought-gap', 0)).label).toBe('Working');
    // Tool arguments were generated during the silence; their first packet ends it
    expect(foldActivity(agentTurn('thought-gap', 1))).toMatchObject({ kind: 'edit', target: 'sample.ts', active: true });
  });

  it('uses the generic label in Chinese', () => {
    setLocale('zh-CN');
    const turn: AgentTurn = { role: 'agent', blocks: [{ type: 'thought', text: 'Preparing a write.', startedAt: 1000, streaming: true }] };
    expect(foldActivity(turn).label).toBe('正在处理');
  });
});
