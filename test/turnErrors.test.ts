import { describe, expect, it } from 'vitest';
import type { AgentTurn, NoticeBlock } from '../src/shared/transcript';
import { isContextLengthError } from '../src/shared/turnErrors';
import { absorbedNotices, hasTurnContent } from '../src/webview/chat/turnOutcome';

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
