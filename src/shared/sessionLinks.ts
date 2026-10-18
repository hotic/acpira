// Session links: `acpira://session/<id>` is the canonical form (tool results, IDEA, the harness); the VS Code shell injects a
// clickable base (`vscode://hotic.acpira/session/`, `cursor://…`) the page copies instead. Parsing is plain string work so the
// Rust mirror (acpira_shared::session_links) agrees on every case of test/fixtures/session-links.json

export const SESSION_LINK_PREFIX = 'acpira://session/';

const RESERVED = new Set(['index', 'prefs', 'categories', 'trash']);

/** A session id names files under the store root: ASCII letters, digits, `_` and `-` only (TranscriptStore's is_session_id) */
export function isSessionId(id: string): boolean {
  return /^[A-Za-z0-9_-]+$/.test(id) && !RESERVED.has(id);
}

/** The link for a session; `base` ends with `/session/` (the page's injected base or the canonical prefix) */
export function sessionLink(id: string, base: string = SESSION_LINK_PREFIX): string {
  return base + id;
}

// Surrounding `<…>` or backticks are what models and chat clients wrap links in
function unwrap(input: string): string {
  const s = input.trim();
  if (s.length >= 2 && ((s.startsWith('<') && s.endsWith('>')) || (s.startsWith('`') && s.endsWith('`')))) return s.slice(1, -1).trim();
  return s;
}

/** The session a link points at: `acpira://session/<id>`, or any `scheme://…/session/<id>` with "acpira" before the path's end
 *  (`vscode://hotic.acpira/session/<id>`); query, fragment and one trailing slash are ignored. Bare ids are not links */
export function sessionIdOfLink(href: string): string | undefined {
  let s = unwrap(href);
  const cut = s.search(/[?#]/);
  if (cut >= 0) s = s.slice(0, cut);
  if (s.endsWith('/')) s = s.slice(0, -1);
  const m = /^([A-Za-z][A-Za-z0-9+.-]*):\/\/(.*)$/.exec(s);
  if (!m) return undefined;
  const scheme = m[1]!.toLowerCase();
  const rest = m[2]!;
  const slash = rest.indexOf('/');
  const authority = slash < 0 ? rest : rest.slice(0, slash);
  const path = slash < 0 ? '' : rest.slice(slash);
  if (scheme === 'acpira' && authority.toLowerCase() === 'session') {
    const id = path.slice(1);
    return isSessionId(id) ? id : undefined;
  }
  // `/session/` must sit in the path, and "acpira" anywhere before it (scheme, authority or an earlier path segment)
  const at = path.lastIndexOf('/session/');
  if (at < 0) return undefined;
  const id = path.slice(at + '/session/'.length);
  const before = `${scheme}://${authority}${path.slice(0, at)}`.toLowerCase();
  return isSessionId(id) && before.includes('acpira') ? id : undefined;
}

/** A session reference as a tool or the composer takes it: a link, else a bare id */
export function sessionRefId(input: string): string | undefined {
  const s = unwrap(input);
  return sessionIdOfLink(s) ?? (isSessionId(s) ? s : undefined);
}

// Acpira's session tools (`acpira mcp` search_sessions / read_session), recognized host-side across the adapters' namings and
// carried in verbKey. Their target is the session as its canonical link first when the call names one, then a read's turn
// range (`3-5`, `3-`, `-5`) or a search's query (normalize.rs session_tool_target)
export function isSessionTool(block: { verbKey?: string }): boolean {
  return block.verbKey === 'verb.searchSessions' || block.verbKey === 'verb.readSession';
}

export function splitSessionTarget(target: string | undefined): { id?: string; rest?: string } {
  const text = (target ?? '').trim();
  const space = text.indexOf(' ');
  const head = space < 0 ? text : text.slice(0, space);
  const id = sessionIdOfLink(head);
  const rest = (id ? (space < 0 ? '' : text.slice(space + 1)) : text).trim();
  return { ...(id ? { id } : {}), ...(rest ? { rest } : {}) };
}

// What a read_session call read, from its range (`3-5`, `3-`, `-5`): no range is the session's outline, an open range
// runs to the last turn, and a closed one lists its turns unless it is too long to list (`span`)
export type SessionRead =
  | { type: 'outline' }
  | { type: 'turns'; turns: number[] }
  | { type: 'from'; from: number }
  | { type: 'span'; from: number; to: number };

export const LISTED_TURNS = 20;

export function sessionRead(range: string | undefined): SessionRead {
  const m = /^(\d*)-(\d*)$/.exec(range ?? '');
  if (!m || (!m[1] && !m[2])) return { type: 'outline' };
  const from = m[1] ? Number(m[1]) : 1;
  if (!m[2]) return { type: 'from', from };
  const lo = Math.min(from, Number(m[2]));
  const hi = Math.max(from, Number(m[2]));
  return hi - lo < LISTED_TURNS ? { type: 'turns', turns: Array.from({ length: hi - lo + 1 }, (_, i) => lo + i) } : { type: 'span', from: lo, to: hi };
}
