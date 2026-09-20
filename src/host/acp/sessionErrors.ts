import * as acp from '@agentclientprotocol/sdk';
import type { Attachment, TurnError } from '@shared/transcript';
import { describeDrafts } from './attachments';
import { msg } from '../errors';
import { t } from '../i18n';

// Credential hand-off by the account layer failed: enters auth_required just like -32000, but the reason must reach the user
export class AccountAuthError extends Error {}

// First line of the text, or what was attached when there is no text (title of a session opened with attachments only)
export function summarizePrompt(text: string, attachments: Attachment[]): string {
  return text.trim().split('\n')[0]!.trim() || describeDrafts(attachments);
}

// Which option auto-approval picks: allow_always first, then allow_once, otherwise the first one
export function bestAllow(options: acp.PermissionOption[]): string {
  const o = options.find(o => o.kind === 'allow_always') ?? options.find(o => o.kind === 'allow_once') ?? options[0];
  if (!o) throw new Error(t('host.noPermissionOptions'));
  return o.optionId;
}

export function isAuth(e: unknown): boolean {
  if (e instanceof AccountAuthError) return true;
  return e instanceof acp.RequestError ? e.code === -32000 : /auth/i.test(msg(e)) && /required|login|unauthor/i.test(msg(e));
}

// What a failed session/prompt leaves on the turn: the JSON-RPC message and code, plus Devin's typed cause (errorKind / retryable) when present.
// Some agents put the readable reason only in data (Devin: data.message or data.detail), so that is preferred over a generic top-level message
export function turnErrorOf(e: unknown): TurnError {
  if (!(e instanceof acp.RequestError)) return { message: msg(e) };
  const data = (e.data && typeof e.data === 'object' ? e.data : {}) as Record<string, unknown>;
  const detail = [data.message, data.detail, data.reason].find((v): v is string => typeof v === 'string' && v.trim().length > 0);
  const kind = data['cognition.ai/errorKind'];
  const retryable = data['cognition.ai/retryable'];
  return {
    message: detail && detail !== e.message ? `${e.message}: ${detail}` : e.message,
    code: e.code,
    ...(typeof kind === 'string' ? { kind } : {}),
    ...(typeof retryable === 'boolean' ? { retryable } : {}),
  };
}

const AUTH_WORDS = /auth|credential|login|logged|unauthor/i;

// Pull a human-readable reason out of one stderr line when it is about authentication. Structured logs (Kimi writes ndjson: {"msg":"acp: auth readiness probe failed…","error":"provider … has no credential configured"})
// yield their error field; plain lines are kept as-is. Anything not about auth yields undefined
export function authHintOf(line: string): string | undefined {
  const text = line.trim();
  if (!text) return undefined;
  // The JSON-RPC layer's own "Sending error response" echo only repackages what the response already carries; it isn't a diagnosis
  if (text.includes('jsonrpc::outgoing_actor')) return undefined;
  // Devin's generic missing-credential warning adds no diagnosis; use the localized login guidance.
  // The complete stderr line remains in the output log.
  if (text.includes('ACP: Creating session without credentials - agent may not work')) return undefined;
  if (text.startsWith('{')) {
    try {
      const j = JSON.parse(text) as Record<string, unknown>;
      const m = typeof j.msg === 'string' ? j.msg : typeof j.message === 'string' ? j.message : '';
      const err = typeof j.error === 'string' ? j.error : typeof j.err === 'string' ? j.err : undefined;
      if (!AUTH_WORDS.test(`${m} ${err ?? ''}`)) return undefined;
      return err ?? (m || undefined);
    } catch { /* not JSON, fall through to plain text */ }
  }
  return AUTH_WORDS.test(text) ? text : undefined;
}

// The peer forgot this session: Devin reports errorKind=session_not_found (empty sessions are swept when the process exits); fall back to matching the message text
export function isSessionGone(e: unknown): boolean {
  if (e instanceof acp.RequestError) {
    const kind = (e.data as Record<string, unknown> | undefined)?.['cognition.ai/errorKind'];
    if (kind === 'session_not_found') return true;
  }
  return /session not found/i.test(msg(e));
}

// How a failed session/resume or session/load ended. `undefined` means the method isn't there at all (no restore path);
// 'gone' the peer doesn't know the session; 'locked' another process holds it; 'unresumable' the peer knows it but can't
// continue it; 'failed' any other error — a connection problem worth a retryable error state, not a missing capability
export type RestoreFailure = 'gone' | 'locked' | 'unresumable' | 'failed';

// DeepSeek Harness (packages/acp/acp/src/index.ts @ 0.1.6-alpha.2) reports every restore problem as a bare
// invalidParams with the reason only in the message: `unknown session: <id>`, `session is already active: <id>`,
// `session is not resumable: <id>`, `session cwd does not match: <cwd>`, and MCP config errors surfaced the same way.
// Grok / Kimi also answer a bare invalidParams for an unknown session id, so an unrecognized -32602 keeps meaning "gone".
export function classifyRestoreError(e: unknown): RestoreFailure | undefined {
  if (isSessionGone(e)) return 'gone';
  if (isSessionLocked(e)) return 'locked';
  if (isMethodMissing(e)) return undefined;
  if (e instanceof acp.RequestError && e.code === -32602) {
    const data = (e.data && typeof e.data === 'object' ? e.data : {}) as Record<string, unknown>;
    const text = `${e.message} ${[data.message, data.detail, data.reason].filter((v): v is string => typeof v === 'string').join(' ')}`;
    if (/already active|in use|held by|locked/i.test(text)) return 'locked';
    if (/not resumable|cannot be resumed/i.test(text)) return 'unresumable';
    if (/\bcwd\b|working directory|absolute path|\bmcp\b/i.test(text)) return 'failed';
    return 'gone';
  }
  return 'failed';
}

// Devin's typed "another process holds this session" (-32015, retryable): real evidence of occupation,
// shown to the user as such instead of a generic restore failure
export function isSessionLocked(e: unknown): boolean {
  return e instanceof acp.RequestError
    && (e.data as Record<string, unknown> | undefined)?.['cognition.ai/errorKind'] === 'session_locked';
}

// The capabilities advertised the feature but the process doesn't actually implement the method
export function isMethodMissing(e: unknown): boolean {
  return e instanceof acp.RequestError && e.code === -32601;
}
