import { AgentBlock, SessionSummary, SessionView, Turn } from '../wire/Wire';
import { t } from './L10n';

// Pure pieces of the session list, its menus and exports: apps/ios Logic/SessionListRows.swift, SessionSearch.swift,
// SessionExport.swift (apps/android logic/ of the same names), and the list's dates and folder names.

const DAY_MS: number = 86400000;
const SSH: string = 'ssh://';

/** Whether the list shows the session's unread mark (a background session carries it beside its ring) */
export function isUnread(s: SessionSummary): boolean {
  return s.state === 'unread' || s.unread === true;
}

function needsAttention(s: SessionSummary): boolean {
  return isUnread(s) || s.state === 'waiting' || s.state === 'error';
}

/** Match LIVE_KEEP: many pure live sessions stay paged, but attention is never capped */
const LIVE_KEEP: number = 5;

/**
 * A project's rows in the by-project list (SessionList.tsx cutRows): the latest `cut`, `extra` more paged out, and past
 * them the sessions needing attention and the open chat, which never hide, plus a small live tail
 */
export class SessionListRows {
  rows: SessionSummary[] = [];
  /** The sessions still hidden behind "N more" */
  rest: SessionSummary[] = [];
  /** Anything pages at all: the "N more" row stands */
  canFold: boolean = false;

  static of(sessions: SessionSummary[], cut: number, extra: number, activeId: string | undefined): SessionListRows {
    const out = new SessionListRows();
    const beyond = sessions.slice(cut + extra);
    const live = beyond.filter((s: SessionSummary) => (s.state === 'working' || s.state === 'background') && !needsAttention(s));
    const kept = new Set<string>(beyond.filter((s: SessionSummary) => needsAttention(s)).map((s: SessionSummary) => s.id));
    if (live.length <= LIVE_KEEP) {
      live.forEach((s: SessionSummary) => kept.add(s.id));
    }
    if (activeId !== undefined && beyond.some((s: SessionSummary) => s.id === activeId)) {
      kept.add(activeId);
    }
    const pageable = sessions.slice(cut).filter((s: SessionSummary) => !kept.has(s.id));
    out.rows = sessions.slice(0, cut).concat(pageable.slice(0, extra)).concat(beyond.filter((s: SessionSummary) => kept.has(s.id)));
    out.rest = pageable.slice(extra);
    out.canFold = pageable.length > 0;
    return out;
  }
}

/** The project's mark, ranked as the desktop's project rows rank it (SessionList.tsx STATE_RANK) */
export class Pressing {
  state: string = '';
  unread: boolean = false;
}

export function pressing(sessions: SessionSummary[]): Pressing | undefined {
  const rank: string[] = ['working', 'waiting', 'error', 'unread', 'background'];
  const state = rank.find((st: string) => sessions.some((s: SessionSummary) => s.state === st));
  if (state === undefined) {
    return undefined;
  }
  const out = new Pressing();
  out.state = state;
  out.unread = state === 'background' && sessions.some((s: SessionSummary) => s.state === 'background' && s.unread === true);
  return out;
}

// The history search (packages/ui chat/sessionSearch.ts)

/** The distinct lower-cased terms of a query */
export function searchTerms(query: string): string[] {
  const out: string[] = [];
  for (const term of query.toLowerCase().split(/\s+/)) {
    if (term.length > 0 && !out.includes(term)) {
      out.push(term);
    }
  }
  return out;
}

/** Every term occurs in one of the row's own words: the title, the agent's name or the project's */
export function searchMatches(fields: string[], terms: string[]): boolean {
  const hay = fields.join('\n').toLowerCase();
  return terms.every((term: string) => hay.includes(term));
}

/** `text` split around every occurrence of the terms, longest first, case-insensitively; matches at the odd indices */
export function searchParts(text: string, terms: string[]): string[] {
  const sorted = terms.filter((x: string) => x.length > 0).sort((a: string, b: string) => b.length - a.length);
  if (sorted.length === 0) {
    return [text];
  }
  const lower = text.toLowerCase();
  const out: string[] = [];
  let plain = '';
  let i = 0;
  while (i < text.length) {
    const hit = sorted.find((term: string) => lower.startsWith(term, i));
    if (hit !== undefined) {
      out.push(plain);
      out.push(text.substring(i, i + hit.length));
      plain = '';
      i += hit.length;
    } else {
      plain += text.charAt(i);
      i++;
    }
  }
  out.push(plain);
  return out;
}

const EXCERPT_LEAD: number = 8;
const EXCERPT_LENGTH: number = 160;

/** The text of a session's prompts and replies, one entry per message */
function messages(view: SessionView): string[] {
  const out: string[] = [];
  for (const turn of view.turns) {
    if (turn.role === 'user') {
      out.push(turn.text ?? '');
    } else {
      (turn.blocks ?? []).filter((b: AgentBlock) => b.type === 'text').forEach((b: AgentBlock) => out.push(b.markdown ?? ''));
    }
  }
  return out;
}

/**
 * The demo's conversation search: every term somewhere in the prompts and replies, the snippet from the first message
 * holding any of them; undefined when the title alone matches or a term is missing
 */
export function searchSnippet(view: SessionView, terms: string[]): string | undefined {
  const said = messages(view);
  const all = said.join('\n').toLowerCase();
  if (!terms.every((term: string) => all.includes(term)) || searchMatches([view.title], terms)) {
    return undefined;
  }
  for (const message of said) {
    const flat = message.split(/[\n\r]+/).filter((l: string) => l.length > 0).join(' ');
    const lower = flat.toLowerCase();
    let from = -1;
    let length = 0;
    for (const term of terms) {
      const at = lower.indexOf(term);
      if (at >= 0 && (from < 0 || at < from)) {
        from = at;
        length = term.length;
      }
    }
    if (from < 0) {
      continue;
    }
    const start = Math.max(0, from - EXCERPT_LEAD);
    const end = Math.min(flat.length, start + EXCERPT_LENGTH);
    return (start > 0 ? '…' : '') + flat.substring(start, Math.max(end, from + length)) + (end < flat.length ? '…' : '');
  }
  return undefined;
}

// Dates

function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** The day caption a session falls under: today, yesterday, week, month or older */
export function recency(iso: string, now: number): string {
  const at = Date.parse(iso);
  if (isNaN(at)) {
    return 'older';
  }
  const today = startOfDay(now);
  if (at >= today) {
    return 'today';
  }
  if (at >= today - DAY_MS) {
    return 'yesterday';
  }
  const days = (now - at) / DAY_MS;
  return days < 7 ? 'week' : days < 30 ? 'month' : 'older';
}

/** Today: the time of day; earlier: month/day, as the UI locale writes them */
export function sessionTime(iso: string, now: number, locale: string): string {
  const at = Date.parse(iso);
  if (isNaN(at)) {
    return '';
  }
  const d = new Date(at);
  if (at >= startOfDay(now)) {
    const h = d.getHours();
    const m = `${d.getMinutes()}`.padStart(2, '0');
    // Chinese and Japanese read the 24-hour clock; English the 12-hour one with its marker
    if (locale.startsWith('zh') || locale.startsWith('ja') || locale.startsWith('ko')) {
      return `${h}:${m}`;
    }
    if (locale.startsWith('en')) {
      return `${h % 12 === 0 ? 12 : h % 12}:${m} ${h < 12 ? 'AM' : 'PM'}`;
    }
    return `${`${h}`.padStart(2, '0')}:${m}`;
  }
  return `${d.getMonth() + 1}/${d.getDate()}`;
}

// Folders

/** A folder's last path part, the name the list shows for its project */
export function projectName(cwd: string): string {
  const parts = cwd.split(/[\/\\]/).filter((p: string) => p.length > 0);
  return parts.length > 0 ? parts[parts.length - 1] : cwd;
}

/** `ssh://<alias>/<path>`'s alias (apps/ios RemoteFolder.parse); undefined for a local folder */
export function remoteAlias(folder: string): string | undefined {
  if (!folder.startsWith(SSH)) {
    return undefined;
  }
  const rest = folder.substring(SSH.length);
  const slash = rest.indexOf('/');
  return slash > 0 ? rest.substring(0, slash) : undefined;
}

function hostPath(folder: string): string {
  const alias = remoteAlias(folder);
  return alias === undefined ? folder : folder.substring(SSH.length + alias.length);
}

/** A folder's own name, on a host its path's */
export function folderLabel(folder: string): string {
  return projectName(hostPath(folder));
}

/** The folder's parent as a short subtitle (NativeProjectMenu.subtitle): its last two parts, `…/` before more */
export function folderSubtitle(folder: string): string {
  const path = hostPath(folder);
  const parts = path.split(/[\/\\]/).filter((p: string) => p.length > 0);
  parts.pop();
  if (parts.length === 0) {
    return path.startsWith('/') ? '/' : '.';
  }
  const tail = parts.slice(-2).join('/');
  return parts.length > 2 ? `…/${tail}` : (path.startsWith('/') ? '/' : '') + tail;
}

/** Folders split by machine: this computer's, and each SSH host's by alias in the order they first turn up */
export class Places {
  local: string[] = [];
  hosts: string[] = [];
  byHost: Map<string, string[]> = new Map<string, string[]>();

  static of(folders: string[]): Places {
    const out = new Places();
    for (const f of folders) {
      if (f.length === 0) {
        continue;
      }
      const alias = remoteAlias(f);
      if (alias === undefined) {
        if (!out.local.includes(f)) {
          out.local.push(f);
        }
        continue;
      }
      const list = out.byHost.get(alias) ?? [];
      if (!list.includes(f)) {
        list.push(f);
      }
      out.byHost.set(alias, list);
      if (!out.hosts.includes(alias)) {
        out.hosts.push(alias);
      }
    }
    return out;
  }
}

// Exports (apps/ios Logic/SessionExport.swift): Markdown for reading, plain text, the view as JSON

function exportBlock(b: AgentBlock): string | undefined {
  switch (b.type) {
    case 'thought':
      return `<details><summary>${t('export.label.thinking')}</summary>\n\n${b.text ?? ''}\n\n</details>`;
    case 'text':
      return b.markdown ?? '';
    case 'tool_call':
      return `- **${b.verb ?? ''}**` + (b.target !== undefined ? ` \`${b.target}\`` : '');
    case 'compaction':
      return `_${t('export.label.compacted')}_`;
    default:
      return undefined;
  }
}

export function exportMarkdown(view: SessionView, agentName: string, exportedAt: number): string {
  const parts: string[] = [
    `# ${view.title}`,
    `- **${t('export.label.agent')}**: ${agentName}\n- **${t('export.label.project')}**: ${view.cwd}\n- **${t('export.label.exported')}**: ${new Date(exportedAt).toISOString()}`,
    '---'
  ];
  for (const turn of view.turns) {
    if (turn.role === 'user') {
      parts.push(`### ${t('export.label.user')}\n\n${turn.text ?? ''}`);
      continue;
    }
    const out: string[] = [`### ${agentName}`];
    for (const b of turn.blocks ?? []) {
      const text = exportBlock(b);
      if (text !== undefined) {
        out.push('');
        out.push(text);
      }
    }
    if (turn.stop !== undefined && turn.stop !== 'end_turn') {
      out.push('');
      out.push(`> ${turn.stop}`);
    }
    parts.push(out.join('\n'));
  }
  return parts.join('\n\n') + '\n';
}

class Said {
  user: boolean = false;
  body: string = '';
}

/** The engine's plain-text export for the demo: the title, then each run of one speaker under its label */
export function exportText(view: SessionView, agentName: string): string {
  const said: Said[] = [];
  const say = (user: boolean, raw: string): void => {
    const body = raw.trim();
    if (body.length === 0) {
      return;
    }
    const last = said.length > 0 ? said[said.length - 1] : undefined;
    if (last !== undefined && last.user === user) {
      last.body = last.body + '\n\n' + body;
    } else {
      const s = new Said();
      s.user = user;
      s.body = body;
      said.push(s);
    }
  };
  view.turns.forEach((turn: Turn) => {
    if (turn.role === 'user') {
      say(true, turn.text ?? '');
    } else {
      (turn.blocks ?? []).filter((b: AgentBlock) => b.type === 'text').forEach((b: AgentBlock) => say(false, b.markdown ?? ''));
    }
  });
  const label = t('export.label.user');
  const parts: string[] = [view.title].concat(said.map((s: Said) => `[${s.user ? label : agentName}]\n${s.body}`));
  return parts.join('\n\n') + '\n';
}

export function exportJson(view: SessionView): string {
  return JSON.stringify(view, null, 2);
}

const FILE_NAME_MAX: number = 60;

/** The title as a file name: path separators and control characters out, at most 60 characters */
export function exportFileName(title: string): string {
  const cleaned = title.replace(/[\/\\:*?"<>|\u0000-\u001f]/g, '-').trim();
  return cleaned.length === 0 ? 'session' : cleaned.substring(0, FILE_NAME_MAX);
}
