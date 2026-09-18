import { describe, expect, it } from 'vitest';
import type { Turn } from '../src/shared/transcript';
import { exportFileName, exportMarkdown } from '../src/shared/exportTranscript';

const turns: Turn[] = [
  { role: 'user', text: 'fix the tests', attachments: [
    { kind: 'image', mimeType: 'image/png', blob: 'h1.png', name: 'a.png' },
    { kind: 'text', name: 'notes.txt', blob: 'h2.txt' },
    { kind: 'file', uri: 'file:///repo/x.ts', name: 'x.ts' },
  ] },
  { role: 'agent', startedAt: 1, endedAt: 2, stop: 'end_turn', blocks: [
    { type: 'thought', text: 'thinking about it' },
    { type: 'text', markdown: 'hello world' },
    { type: 'plan', changed: true, entries: [
      { title: 'done thing', status: 'completed' },
      { title: 'doing thing', status: 'in_progress' },
      { title: 'later thing', status: 'pending' },
    ] },
    { type: 'tool_call', id: 't1', kind: 'execute', verb: 'Run', target: 'pnpm test', meta: '3s', status: 'completed',
      content: { type: 'text', text: 'all green' } },
    { type: 'tool_call', id: 't2', kind: 'edit', verb: 'Edit', status: 'completed',
      content: { type: 'diff', lines: [
        { kind: 'hunk', text: '2 unchanged lines' },
        { kind: 'ctx', text: ' a' },
        { kind: 'del', text: '-b' },
        { kind: 'add', text: '+B' },
      ] } },
    { type: 'tool_call', id: 't3', kind: 'read', verb: 'Read', status: 'completed',
      content: { type: 'list', items: ['one', 'two'] } },
    { type: 'question', id: 'q1', questions: [{ id: 'q1', text: 'which?', kind: 'single', options: [] }],
      outcome: 'answered', answers: { q1: ['a', 'b'] } },
    { type: 'question', id: 'q2', questions: [{ id: 'q2', text: 'pending?', kind: 'single', options: [] }] },
    { type: 'permission', id: 'p1', title: 'Run?', options: [] },
    { type: 'compaction', id: 'c1', status: 'completed' },
    { type: 'plan_document', id: 'pd1', title: 'Plan A', markdown: 'do it', toolCallId: 't9', status: 'approved' },
  ] },
  { role: 'user', text: '/compact', auto: true },
  { role: 'agent', startedAt: 3, endedAt: 4, stop: 'error', error: { message: 'boom' }, blocks: [
    { type: 'tool_call', id: 't4', kind: 'execute', verb: 'Run', status: 'completed',
      content: { type: 'text', text: 'x'.repeat(4100) } },
  ] },
];

describe('exportMarkdown', () => {
  const md = exportMarkdown({ title: 'fix the tests', agentName: 'Fake', cwd: '/repo', exportedAt: '2026-09-18T09:05:07Z', turns });

  it('opens with the title, meta bullet lines and a separator', () => {
    expect(md).toContain('# fix the tests');
    expect(md).toContain('- **Agent**: Fake');
    expect(md).toContain('- **Project**: /repo');
    expect(md).toContain('- **Exported**: 2026-09-18T09:05:07Z');
    expect(md).toContain('---');
  });

  it('renders user turns with their attachment names and the auto-compact line', () => {
    expect(md).toContain('### User');
    expect(md).toContain('fix the tests');
    expect(md).toContain('> Attachments: a.png, notes.txt, x.ts');
    expect(md).toContain('_Automatic /compact_');
  });

  it('renders agent blocks: thought, prose, plan, tools, question, compaction, plan doc', () => {
    expect(md).toContain('### Fake');
    expect(md).toContain('<details><summary>Thinking</summary>');
    expect(md).toContain('thinking about it');
    expect(md).toContain('hello world');
    expect(md).toContain('- [x] done thing');
    expect(md).toContain('- [ ] doing thing _(in progress)_');
    expect(md).toContain('- [ ] later thing');
    expect(md).toContain('- **Run** `pnpm test` (3s)');
    expect(md).toContain('```diff\n@@ 2 unchanged lines @@\n a\n-b\n+B\n```');
    expect(md).toContain('  - one\n  - two');
    expect(md).toContain('**Q:** which?');
    expect(md).toContain('**A:** a, b');
    // Pending questions and permission requests are not part of the written record
    expect(md).not.toContain('pending?');
    expect(md).not.toContain('Run?');
    expect(md).toContain('_Context compacted_');
    expect(md).toContain('#### Plan A');
    expect(md).toContain('do it');
  });

  it('caps tool output at 4000 chars and quotes the turn error', () => {
    expect(md).toContain('… (truncated)');
    expect(md).not.toContain('x'.repeat(4100));
    expect(md).toContain('> Error: boom');
  });

  it('closes every code fence it opens', () => {
    const fences = md.match(/```/g) ?? [];
    expect(fences.length).toBeGreaterThan(0);
    expect(fences.length % 2).toBe(0);
  });
});

describe('exportFileName', () => {
  const d = new Date(2026, 8, 18, 9, 5, 7);

  it('strips filesystem-hostile characters and stamps the time', () => {
    expect(exportFileName('Fix: a/b?', 'markdown', d)).toBe('Fix-ab-20260918-090507.md');
    expect(exportFileName('中文 标题', 'json', d)).toBe('中文-标题-20260918-090507.json');
  });

  it('falls back to session for an empty title and caps the slug at 60 code points', () => {
    expect(exportFileName('', 'json', d)).toBe('session-20260918-090507.json');
    expect(exportFileName('x'.repeat(200), 'markdown', d)).toBe(`${'x'.repeat(60)}-20260918-090507.md`);
  });
});
