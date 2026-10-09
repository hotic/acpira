import { afterEach, describe, expect, it } from 'vitest';
import type { Turn } from '../src/shared/transcript';
import { setLocale } from '../src/webview/i18n';
import { goalDuration, goalSpend, lastTypedPrompt } from '../src/webview/chat/goalText';
import { hasTurnContent } from '../src/webview/chat/turnOutcome';

afterEach(() => setLocale('en'));

describe('goal texts', () => {
  it('formats elapsed goal time in seconds, minutes and hours', () => {
    expect(goalDuration(0)).toBe('0s');
    expect(goalDuration(45)).toBe('45s');
    expect(goalDuration(372)).toBe('6m 12s');
    expect(goalDuration(360)).toBe('6m');
    expect(goalDuration(7500)).toBe('2h 05m');
    setLocale('zh-CN');
    expect(goalDuration(7500)).toBe('2 小时 05 分钟');
  });
  it('shows the spend against the budget when there is one', () => {
    expect(goalSpend({})).toBeUndefined();
    expect(goalSpend({ tokensUsed: 1200 })).toBe('1.2K tokens');
    expect(goalSpend({ tokensUsed: 184_000, tokenBudget: 500_000 })).toBe('184K / 500K tokens');
  });
});

describe('"Set as goal" prompt pick', () => {
  const user = (text: string, auto?: boolean): Turn => ({ role: 'user', text, ...(auto ? { auto } : {}) });
  const agent: Turn = { role: 'agent', blocks: [{ type: 'text', markdown: 'ok' }] };
  it('takes the newest typed prompt and skips automatic turns', () => {
    expect(lastTypedPrompt([user('fix the build'), agent])).toBe(0);
    expect(lastTypedPrompt([user('a'), agent, user('b'), agent, user('continue', true), agent])).toBe(2);
  });
  it('offers nothing under a slash command, an empty prompt or an empty thread', () => {
    expect(lastTypedPrompt([user('fix'), agent, user('/goal fix'), agent])).toBeUndefined();
    expect(lastTypedPrompt([user('  '), agent])).toBeUndefined();
    expect(lastTypedPrompt([])).toBeUndefined();
  });
  it('a turn holding only goal milestones has no content to copy or fork', () => {
    expect(hasTurnContent({ role: 'agent', blocks: [{ type: 'goal', event: 'paused' }] })).toBe(false);
  });
});
