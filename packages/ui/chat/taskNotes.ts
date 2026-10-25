import type { BlockProps } from 'streamdown';

type Pluggable = NonNullable<BlockProps['rehypePlugins']>[number];

// Claude Code tells the model about a finished background task with a user-role `<task-notification>` message
// (`<task-id>`, `<status>`, `<summary>`). The model sometimes imitates one at the end of its own reply, right after
// saying it will be notified, while the task is still running (claude-agent-acp, stored transcripts 2026-10-25: both
// cases were model text in the native log, the real notification never reaches the transcript as text). Rendered as
// raw HTML, sanitize dropped the unknown tags but kept their text and the bare `<summary>`, which drew a disclosure
// marker that opens nothing. The block becomes one element here, rendered as a row that says who wrote it.
export const TASK_NOTE_TAG = 'acpira-task-note';

const FIELDS = { 'task-id': 'dataTask', status: 'dataStatus', summary: 'dataSummary' } as const;

interface HastNode {
  type: string;
  tagName?: string;
  value?: string;
  properties?: Record<string, unknown>;
  children?: HastNode[];
}

function textOf(node: HastNode): string {
  if (node.type === 'text') return node.value ?? '';
  return node.children?.map(textOf).join('') ?? '';
}

// The fields of one notification, read wherever they sit inside it (a half-streamed one may lack some)
function fieldsOf(node: HastNode, out: Record<string, string>): Record<string, string> {
  for (const child of node.children ?? []) {
    const key = child.tagName && FIELDS[child.tagName as keyof typeof FIELDS];
    if (key) out[key] = textOf(child).trim();
    else fieldsOf(child, out);
  }
  return out;
}

function walk(node: HastNode): void {
  const children = node.children;
  if (!children) return;
  for (let i = 0; i < children.length; i++) {
    const child = children[i]!;
    if (child.tagName === 'task-notification') {
      children[i] = { type: 'element', tagName: TASK_NOTE_TAG, properties: fieldsOf(child, {}), children: [] };
      // Written without a blank line before it, the block is inline HTML inside a paragraph, which may not hold the row's div
      if (node.tagName === 'p') node.tagName = 'div';
    } else walk(child);
  }
}

// Puts each notification in an HTML block of its own: open tag alone on its line, no blank line inside, a blank line
// on both sides. Inside a paragraph it would be inline HTML, and the HTML parser closes the paragraph at `<summary>`,
// pushing the summary out of the notification. A half-streamed one (no closing tag yet) runs to the end
const NOTE_SOURCE = /\s*<task-notification>([\s\S]*?)(<\/task-notification>|$)\s*/g;
export function isolateTaskNotes(markdown: string): string {
  if (!markdown.includes('<task-notification>')) return markdown;
  return markdown.replace(NOTE_SOURCE, (_, body: string, end: string) =>
    `\n\n<task-notification>\n${body.replace(/\n\s*\n/g, '\n').trim()}\n${end}${end ? '\n\n' : ''}`).trim();
}

// Runs after rehype-raw and before sanitize
export function rewriteTaskNotes() {
  return (tree: HastNode) => walk(tree);
}

// streamdown's sanitize step with the note element and its fields let through
export function withTaskNotes(sanitize: Pluggable): Pluggable {
  if (!Array.isArray(sanitize)) throw new Error('streamdown sanitize plugin has no schema');
  const [plugin, schema] = sanitize as [unknown, { tagNames?: string[]; attributes?: Record<string, unknown[]> }];
  return [plugin, {
    ...schema,
    tagNames: [...(schema.tagNames ?? []), TASK_NOTE_TAG],
    attributes: { ...schema.attributes, [TASK_NOTE_TAG]: Object.values(FIELDS) },
  }] as Pluggable;
}
