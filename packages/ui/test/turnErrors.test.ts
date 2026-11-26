import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { AgentTurn, NoticeBlock } from '@acpira/shared/transcript';
import { isContextLengthError } from '@acpira/shared/turnErrors';
import { absorbedNotices, hasTurnContent, hasTurnStats, showsTurnActions } from '../chat/turnOutcome';
import { NoticeRow } from '../chat/Turns';

// claude-agent-acp, stored transcript 2026-10-04: a session-scoped connection notice offering new_session, then the
// prompt failed with -32000 and no AIR failure behind it
const lost: NoticeBlock = { type: 'notice', id: 's:session-error:1', revision: 1, category: 'connection', severity: 'error', title: 'The connection to Claude was lost.', actions: ['new_session'] };
const authFailed: AgentTurn = { role: 'agent', stop: 'error', error: { message: 'Authentication required', code: -32000 }, blocks: [lost] };

describe('failure notices on an errored turn', () => {
  it('folds error notices into the outcome of a turn that ended on its own error', () => {
    expect(absorbedNotices(authFailed)).toEqual([lost]);
  });

  it('keeps notices standing when an AIR failure owns the turn error, for warnings, or without an error', () => {
    expect(absorbedNotices({ ...authFailed, error: { message: 'x', failureId: lost.id, actions: [] } })).toEqual([]);
    expect(absorbedNotices({ ...authFailed, blocks: [{ ...lost, severity: 'warning' }] })).toEqual([]);
    expect(absorbedNotices({ ...authFailed, stop: 'end_turn', error: undefined })).toEqual([]);
  });

  it('offers no turn actions for a turn holding only failure notices', () => {
    expect(hasTurnContent(authFailed)).toBe(false);
    expect(hasTurnContent({ ...authFailed, blocks: [lost, { type: 'text', markdown: 'partial' }] })).toBe(true);
  });

  // claude-agent-acp, stored transcript 2026-10-30: a stop pressed before anything streamed leaves an empty turn
  it('keeps the stats of a turn stopped before anything streamed, not of one that failed', () => {
    const stopped: AgentTurn = { role: 'agent', stop: 'cancelled', blocks: [], startedAt: 0, endedAt: 680 };
    expect(hasTurnStats(stopped)).toBe(true);
    expect(hasTurnStats({ ...stopped, startedAt: undefined, endedAt: undefined })).toBe(false);
    expect(hasTurnStats({ ...authFailed, startedAt: 0, endedAt: 680 })).toBe(false);
    expect(showsTurnActions(stopped, false)).toBe(true);
    expect(showsTurnActions(authFailed, false)).toBe(false);
    expect(showsTurnActions(authFailed, true)).toBe(true);
  });
});

describe('context overflow detection', () => {
  it.each([
    'The prompt to the model was too long. Try reducing the size of your context (including any rules, skills, etc.).',
    'Prompt is too long',
    'Context length exceeded',
    'This model has a maximum context length of 128000 tokens.',
  ])('recognizes the model input failure: %s', message => {
    expect(isContextLengthError({ message, kind: 'internal', retryable: true })).toBe(true);
  });

  it('accepts a structured context error without changing vendor metadata', () => {
    const error = { message: 'Bad request', kind: 'context_length_exceeded', retryable: true };
    expect(isContextLengthError(error)).toBe(true);
    expect(error.retryable).toBe(true);
  });

  it.each(['Tool output was too long', 'Maximum output tokens reached', 'Rate limit exceeded', 'Request took too long'])('keeps unrelated failures on their existing path: %s', message => {
    expect(isContextLengthError({ message })).toBe(false);
  });
});

describe('NoticeRow wrapping and truncation', () => {
  const renderNotice = (block: NoticeBlock) => renderToStaticMarkup(createElement(NoticeRow, { block }));

  it('wraps bare notice titles without shrink-0 and aligns the lead icon to the first line', () => {
    const html = renderNotice({
      type: 'notice',
      id: 'notice:1',
      revision: 1,
      category: 'unknown',
      severity: 'warning',
      title: '`[features].multi_agent` is deprecated. It still controls the V1 fallback.',
      actions: [],
    });
    expect(html).toContain('items-start lead-top');
    const span = html.match(/<span class="[^"]*">[^<]*multi_agent[^<]*<\/span>/)?.[0] ?? '';
    expect(span).toContain('whitespace-pre-wrap');
    expect(span).toContain('[overflow-wrap:anywhere]');
    expect(span).not.toContain('shrink-0');
    expect(span).not.toContain('whitespace-nowrap');
  });

  it('truncates foldable notice titles without shrink-0 blocking flex shrinking', () => {
    const html = renderNotice({
      type: 'notice',
      id: 'notice:2',
      revision: 1,
      category: 'unknown',
      severity: 'warning',
      title: 'A very long warning title that must truncate cleanly instead of overflowing the disclosure row',
      details: 'Extra details line',
      actions: [],
    });
    const span = html.match(/<span class="[^"]*" title="[^"]*">A very long warning title[^<]*<\/span>/)?.[0] ?? '';
    expect(span).toContain('min-w-0 truncate');
    expect(span).not.toContain('shrink-0');
  });
});
