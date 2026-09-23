// Transcript → a file the user can open or share: Markdown for reading, JSON for tooling. Pure and DOM-free so both
// tsconfigs (host and webview) can type-check it; the host formats, writes under exports/, and opens the result

import type { AgentBlock, ToolContent, Turn } from './transcript';

// Every user-facing label the export emits; English is the default so a host without a locale still produces a readable file
export interface ExportLabels {
  user: string;
  agent: string;
  project: string;
  exported: string;
  attachments: string;
  thinking: string;
  compacted: string;
  autoCompact: string;
  error: string;
}

export const EXPORT_LABELS_EN: ExportLabels = {
  user: 'User',
  agent: 'Agent',
  project: 'Project',
  exported: 'Exported',
  attachments: 'Attachments',
  thinking: 'Thinking',
  compacted: 'Context compacted',
  autoCompact: 'Automatic /compact',
  error: 'Error',
};

export interface ExportInput {
  title: string;
  agentName: string;
  cwd: string;
  exportedAt: string;
  turns: Turn[];
}

const TOOL_CONTENT_MAX = 4000;

// Resolves a blob name to its absolute store path so an agent image can export as a real file link;
// absent (a store without disk paths) the image degrades to a `[image]` note
export type ExportBlobPath = (name: string) => string | undefined;

export function exportMarkdown(input: ExportInput, labels: ExportLabels = EXPORT_LABELS_EN, blobPath?: ExportBlobPath): string {
  const parts: string[] = [
    `# ${input.title}`,
    `- **${labels.agent}**: ${input.agentName}\n- **${labels.project}**: ${input.cwd}\n- **${labels.exported}**: ${input.exportedAt}`,
    '---',
  ];
  for (const turn of input.turns) {
    const rendered = turn.role === 'user' ? userTurn(turn, labels) : agentTurn(turn, input.agentName, labels, blobPath);
    if (rendered) parts.push(rendered);
  }
  return `${parts.join('\n\n')}\n`;
}

function userTurn(turn: Extract<Turn, { role: 'user' }>, labels: ExportLabels): string {
  if (turn.auto) return `_${labels.autoCompact}_`;
  const out = [`### ${labels.user}`, '', turn.text];
  if (turn.attachments?.length) {
    const names = turn.attachments.map(a => a.kind === 'file' ? a.name : a.name ?? a.blob ?? 'image');
    out.push('', `> ${labels.attachments}: ${names.join(', ')}`);
  }
  return out.join('\n');
}

function agentTurn(turn: Extract<Turn, { role: 'agent' }>, agentName: string, labels: ExportLabels, blobPath?: ExportBlobPath): string {
  const out = [`### ${agentName}`];
  for (const b of turn.blocks) {
    const rendered = block(b, labels, blobPath);
    if (rendered) out.push('', rendered);
  }
  if (turn.stop === 'error' && turn.error) out.push('', `> ${labels.error}: ${turn.error.message}`);
  else if (turn.stop && turn.stop !== 'end_turn') out.push('', `> ${turn.stop}`);
  return out.join('\n');
}

function block(b: AgentBlock, labels: ExportLabels, blobPath?: ExportBlobPath): string | undefined {
  switch (b.type) {
    case 'thought':
      return `<details><summary>${labels.thinking}</summary>\n\n${b.text}\n\n</details>`;
    case 'text':
      return b.markdown;
    case 'image':
      return imageLine(b, blobPath);
    case 'plan':
      return b.entries.map(e => e.status === 'completed' ? `- [x] ${e.title}`
        : `- [ ] ${e.title}${e.status === 'in_progress' ? ' _(in progress)_' : ''}`).join('\n');
    case 'tool_call': {
      const head = `- **${b.verb}**${b.target ? ` \`${b.target}\`` : ''}${b.meta ? ` (${b.meta})` : ''}`;
      const content = (b.contents ?? (b.content ? [b.content] : [])).map(c => toolContent(c, blobPath)).filter(Boolean).join('\n');
      return content ? `${head}\n${content}` : head;
    }
    case 'permission':
      return undefined;
    case 'question': {
      if (!b.outcome) return undefined;
      return b.questions.map(q => {
        const a = b.answers?.[q.id];
        const answer = Array.isArray(a) ? a.join(', ') : a ?? '—';
        return `**Q:** ${q.text}\n\n**A:** ${answer}`;
      }).join('\n\n');
    }
    case 'compaction':
      return b.status === 'completed' ? `_${labels.compacted}_` : `_${labels.compacted}: ${b.status}_`;
    case 'plan_document':
      return `#### ${b.title}\n\n${b.markdown}`;
    default:
      return undefined;
  }
}

// An agent image exports as a link to its blob file on disk; uri-only ones keep the path the agent saved
function imageLine(i: { blob?: string; mimeType: string; uri?: string }, blobPath?: ExportBlobPath): string {
  const path = i.blob && blobPath?.(i.blob);
  if (path) return `![image](${path})`;
  if (i.uri) return `[image](${i.uri})`;
  return `[image: ${i.mimeType}]`;
}

function toolContent(c: ToolContent | undefined, blobPath?: ExportBlobPath): string | undefined {
  if (!c) return undefined;
  switch (c.type) {
    case 'text': {
      const body = c.text.length > TOOL_CONTENT_MAX ? `${c.text.slice(0, TOOL_CONTENT_MAX)}\n… (truncated)` : c.text;
      return `\`\`\`\n${body}\n\`\`\``;
    }
    case 'diff':
      // DiffLine.text already carries the + / - / space prefix; a hunk line's text is a localized note like "2 unchanged lines"
      return `\`\`\`diff\n${c.lines.map(l => l.kind === 'hunk' ? `@@ ${l.text} @@` : l.text).join('\n')}\n\`\`\``;
    case 'list':
      return c.items.map(i => `  - ${i}`).join('\n');
    case 'image':
      return imageLine(c, blobPath);
    default:
      return undefined;
  }
}

// `<slug>-<yyyyMMdd-HHmmss>.<md|json>`: title minus filesystem-hostile characters, whitespace collapsed to dashes,
// capped at 60 code points (CJK survives — only ASCII separators and control characters are dropped)
export function exportFileName(title: string, format: 'markdown' | 'json', now: Date): string {
  const slug = [...title
    .replace(/[/\\:*?"<>|\p{Cc}]/gu, '')
    .replace(/\s+/g, '-')
    .replace(/^[-.]+|[-.]+$/g, '')]
    .slice(0, 60).join('').replace(/[-.]+$/, '') || 'session';
  const p = (n: number) => String(n).padStart(2, '0');
  const stamp = `${now.getFullYear()}${p(now.getMonth() + 1)}${p(now.getDate())}-${p(now.getHours())}${p(now.getMinutes())}${p(now.getSeconds())}`;
  return `${slug}-${stamp}.${format === 'markdown' ? 'md' : 'json'}`;
}
