import * as acp from '@agentclientprotocol/sdk';
import { describe, expect, it } from 'vitest';
import { classifyRestoreError } from '../src/host/acp/sessionErrors';

describe('classifyRestoreError', () => {
  it('DeepSeek Harness invalidParams reasons classify by their message text', () => {
    // packages/acp/acp/src/index.ts @ 0.1.6-alpha.2 reports all of these as a bare -32602
    expect(classifyRestoreError(acp.RequestError.invalidParams(undefined, 'unknown session: s1'))).toBe('gone');
    expect(classifyRestoreError(acp.RequestError.invalidParams(undefined, 'session is already active: s1'))).toBe('locked');
    expect(classifyRestoreError(acp.RequestError.invalidParams(undefined, 'session is not resumable: s1'))).toBe('unresumable');
    expect(classifyRestoreError(acp.RequestError.invalidParams(undefined, 'session cwd does not match: /repo'))).toBe('failed');
    expect(classifyRestoreError(acp.RequestError.invalidParams(undefined, 'mcp server "fs": command not found'))).toBe('failed');
  });

  it('a bare invalidParams with an unrecognized reason still means the session is gone (Grok / Kimi)', () => {
    expect(classifyRestoreError(acp.RequestError.invalidParams({ sessionId: 's1' }, 'unknown session'))).toBe('gone');
  });

  it("Devin's typed errors: session_not_found is gone, session_locked is locked", () => {
    expect(classifyRestoreError(new acp.RequestError(-32016, 'Session not found', { 'cognition.ai/errorKind': 'session_not_found' }))).toBe('gone');
    expect(classifyRestoreError(new acp.RequestError(-32015, 'Session is locked', { 'cognition.ai/errorKind': 'session_locked' }))).toBe('locked');
  });

  it('method-not-found means the agent has no restore path at all', () => {
    expect(classifyRestoreError(acp.RequestError.methodNotFound('session/resume'))).toBeUndefined();
  });

  it('anything else — including a transport-level failure — is a retryable failure', () => {
    expect(classifyRestoreError(acp.RequestError.internalError(undefined, 'transient restore failure'))).toBe('failed');
    expect(classifyRestoreError(new Error('connection lost'))).toBe('failed');
  });
});
