import { describe, expect, it } from 'vitest';
import { failureOf, failureTurnError } from '../src/host/acp/sessionFailure';

const meta = (sessionFailure: unknown) => ({ jetbrains: { air: { version: 1, sessionFailure } } });
const valid = { id: 'turn-1:error', revision: 3, category: 'limit', severity: 'error', title: 'Rate limit exceeded', actions: ['retry'] };

describe('sessionFailure', () => {
  it('decodes a well-formed payload, trimming strings and keeping the declared actions', () => {
    expect(failureOf(meta({ ...valid, details: ' try later ' })))
      .toEqual({ id: 'turn-1:error', revision: 3, category: 'limit', severity: 'error', title: 'Rate limit exceeded', details: 'try later', actions: ['retry'] });
    expect(failureOf(meta({ ...valid, details: undefined, reason: 'quota gone' }))?.details).toBe('quota gone');
    expect(failureOf(meta(valid))?.details).toBeUndefined();
  });

  it('returns undefined when the meta carries no sessionFailure', () => {
    expect(failureOf(undefined)).toBeUndefined();
    expect(failureOf({})).toBeUndefined();
    expect(failureOf(meta(undefined))).toBeUndefined();
    expect(failureOf({ jetbrains: { air: { version: 1 } } })).toBeUndefined();
  });

  it('rejects payloads with missing or malformed required fields, logging each reason', () => {
    const logs: string[] = [];
    const log = (l: string) => logs.push(l);
    expect(failureOf(meta('oops'), log)).toBeUndefined();
    expect(failureOf(meta({ ...valid, id: '' }), log)).toBeUndefined();
    expect(failureOf(meta({ ...valid, id: 7 }), log)).toBeUndefined();
    expect(failureOf(meta({ ...valid, title: '  ' }), log)).toBeUndefined();
    expect(failureOf(meta({ ...valid, revision: 0 }), log)).toBeUndefined();
    expect(failureOf(meta({ ...valid, revision: 1.5 }), log)).toBeUndefined();
    expect(failureOf(meta({ ...valid, revision: '2' }), log)).toBeUndefined();
    expect(failureOf(meta({ ...valid, severity: 'fatal' }), log)).toBeUndefined();
    expect(failureOf(meta({ ...valid, actions: 'retry' }), log)).toBeUndefined();
    expect(logs.length).toBe(9);
    expect(logs.every(l => l.startsWith('sessionFailure ignored'))).toBe(true);
  });

  it('degrades an unknown category to unknown and filters unknown or duplicate actions', () => {
    expect(failureOf(meta({ ...valid, category: 'quota', actions: ['retry', 'cry', 'retry', 7] })))
      .toMatchObject({ category: 'unknown', actions: ['retry'] });
    expect(failureOf(meta({ ...valid, category: undefined }))?.category).toBe('unknown');
    expect(failureOf(meta({ ...valid, actions: [] }))?.actions).toEqual([]);
  });

  it('failureTurnError carries the id, the declared actions and retryable from the retry action', () => {
    const f = failureOf(meta(valid))!;
    expect(failureTurnError(f)).toEqual({
      message: 'Rate limit exceeded', kind: 'limit', retryable: true,
      failureId: 'turn-1:error', actions: ['retry'],
    });
    const noActions = failureTurnError(failureOf(meta({ ...valid, actions: [] }))!);
    expect(noActions.retryable).toBe(false);
    // An explicit empty list, so the alert card shows no generic Retry / Reconnect for it
    expect(noActions.actions).toEqual([]);
    const withDetails = failureTurnError(failureOf(meta({ ...valid, details: 'wait a minute' }))!);
    expect(withDetails.message).toBe('Rate limit exceeded\nwait a minute');
  });
});
