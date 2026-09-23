import type { FailureAction, FailureCategory, TurnError } from '@shared/transcript';

// JetBrains AIR sessionFailure extension (codex-acp / claude-agent-acp): structured warnings and
// errors the adapter reports at `_meta.jetbrains.air.sessionFailure`, on session_info_update
// notifications and on the session/prompt response. Same id + higher revision updates in place;
// a same or lower revision is a duplicate the sender already knows about. Actions are what the
// adapter offers, never inferred from the category.
export interface SessionFailure {
  id: string;
  revision: number;
  category: FailureCategory;
  severity: 'warning' | 'error';
  title: string;
  details?: string;
  actions: FailureAction[];
}

const CATEGORIES = new Set<FailureCategory>(['connection', 'access', 'limit', 'request', 'service', 'unknown']);
const ACTIONS = new Set<FailureAction>(['retry', 'login', 'new_session']);

const text = (v: unknown): string | undefined => typeof v === 'string' && v.trim() ? v.trim() : undefined;

// The strict read of one payload: required fields missing or malformed → undefined (the caller logs).
// An unrecognized category keeps the rest of the payload and degrades to 'unknown'; unknown actions
// are filtered, never guessed
export function failureOf(meta: unknown, log?: (line: string) => void): SessionFailure | undefined {
  const air = (meta as { jetbrains?: { air?: Record<string, unknown> } } | null | undefined)?.jetbrains?.air;
  const raw = air?.sessionFailure;
  if (raw === undefined || raw === null) return undefined;
  const fail = (why: string): undefined => { log?.(`sessionFailure ignored (${why}): ${JSON.stringify(raw)}`); return undefined; };
  if (typeof raw !== 'object') return fail('not an object');
  const p = raw as Record<string, unknown>;
  const id = text(p.id);
  const title = text(p.title);
  const revision = typeof p.revision === 'number' && Number.isInteger(p.revision) && p.revision > 0 ? p.revision : undefined;
  const severity = p.severity === 'warning' || p.severity === 'error' ? p.severity : undefined;
  if (!id) return fail('id missing');
  if (revision === undefined) return fail(`revision not a positive integer: ${String(p.revision)}`);
  if (!severity) return fail(`severity invalid: ${String(p.severity)}`);
  if (!title) return fail('title missing');
  if (!Array.isArray(p.actions)) return fail('actions not an array');
  const category: FailureCategory = typeof p.category === 'string'
    ? (CATEGORIES.has(p.category as FailureCategory) ? p.category as FailureCategory : 'unknown')
    : 'unknown';
  const actions = [...new Set(p.actions.filter((a): a is FailureAction => ACTIONS.has(a as FailureAction)))];
  const details = text(p.details) ?? text(p.reason);
  return { id, revision, category, severity, title, ...(details ? { details } : {}), actions };
}

// The TurnError a turn-ending failure settles with: title (+ details) as the message, the category
// as the kind the alert card's copy line already prints, and the declared actions so the UI shows
// exactly the buttons the adapter offered
export function failureTurnError(f: SessionFailure): TurnError {
  return {
    message: f.details ? `${f.title}\n${f.details}` : f.title,
    kind: f.category,
    retryable: f.actions.includes('retry'),
    failureId: f.id,
    // Always present, even empty: an empty list means the adapter offers no remedy (quota, invalid
    // request), and the alert card must not fall back to its generic Retry / Reconnect pair
    actions: f.actions,
  };
}
