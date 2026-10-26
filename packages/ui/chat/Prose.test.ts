import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { Prose } from './Prose';
import { CodeBlock } from './CodeBlock';
import { fenceLanguage } from './codeSyntax';
import { QuietEntranceContext } from '../ui/Row';
import { OpenSessionContext, OpenToolFileContext } from './fileLinks';

const prose = (markdown: string) => renderToStaticMarkup(createElement(Prose, { block: { type: 'text', markdown } as never }));

describe('markdown prose', () => {
  it('closes bold runs that end on CJK punctuation directly before a CJK letter', () => {
    // CommonMark alone leaves `**…。**范围` literal: the closer is preceded by punctuation and followed by a letter
    const html = prose('老板，**以后不会再卡住；但没有改成免 Passkey。**范围得说清楚');
    // streamdown renders strong as a marked span
    expect(html).toContain('data-streamdown="strong">以后不会再卡住；但没有改成免 Passkey。</span>范围');
    expect(html).not.toContain('**');
  });

  it('shows a still-streaming reply at once when its thread mounts quietly', () => {
    // A session reopened while its turn is held open: the reply was already on screen and must not replay
    const block = { type: 'text', markdown: '计划草稿已经落盘', streaming: true } as never;
    const mount = (quiet: boolean) => renderToStaticMarkup(createElement(QuietEntranceContext.Provider, { value: { current: quiet } }, createElement(Prose, { block })));
    expect(mount(true).replace(/<[^>]+>/g, '')).toBe('计划草稿已经落盘');
    // A reply that starts streaming in front of the reader is still paced in from its first character
    expect(mount(false)).toBe('');
  });

  // acpira:// and vscode:// hrefs are rewritten before sanitize / harden, which would paint them ` [blocked]`
  it('renders session links as clickable links, in Markdown links and in backticks', () => {
    const md = [
      'See [the plan](acpira://session/abc-1) and [VS Code](vscode://hotic.acpira/session/def_2).',
      'Inline `[Wrapped](acpira://session/ghi)` and `acpira://session/jkl`.',
      'Web [site](https://example.com), file [a.ts](file:///repo/a.ts#L3), other <vscode://other.ext/x>.',
    ].join('\n\n');
    const html = renderToStaticMarkup(createElement(OpenSessionContext.Provider, { value: () => {} },
      createElement(OpenToolFileContext.Provider, { value: () => {} }, createElement(Prose, { block: { type: 'text', markdown: md } as never }))));
    expect(html).toContain('href="#acpira-session:abc-1"');
    expect(html).toContain('href="#acpira-session:def_2"');
    expect(html).toMatch(/<code[^>]*role="link"[^>]*>Wrapped<\/code>/);
    expect(html).toMatch(/<code[^>]*role="link"[^>]*title="jkl"[^>]*>acpira:\/\/session\/jkl<\/code>/);
    expect(html).toContain('href="https://example.com/"');
    expect(html).toContain('href="#acpira-file:%2Frepo%2Fa.ts:3"');
    expect(html.match(/\[blocked\]/g)?.length ?? 0).toBe(1);
  });

  // Claude imitates a background task's notification at the end of a reply (stored transcript 2026-10-25)
  it('renders a task notification the model wrote as one row, not loose text and a dead disclosure', () => {
    const tag = '<task-notification>\n<task-id>bohum1hs5</task-id>\n<status>completed</status>\n<summary>Background command completed (exit code 0)</summary>\n</task-notification>';
    for (const md of [`跑完会自动通知我。\n\n${tag}`, `跑完会自动通知我。\n${tag}`, `跑完会自动通知我。${tag.replace(/\n/g, '')}`, `跑完会自动通知我。\n\n${tag.replace(/\n/g, '\n\n')}\n然后继续。`]) {
      const html = prose(md);
      expect(html).toContain('跑完会自动通知我。');
      expect(html).toMatch(/data-task-note[\s\S]*Background command completed \(exit code 0\)[\s\S]*bohum1hs5/);
      expect(html).not.toMatch(/<summary|<p[^>]*>\s*<div/);
      expect(html.match(/bohum1hs5/g)).toHaveLength(1);
      if (md.endsWith('然后继续。')) expect(html).toMatch(/bohum1hs5[\s\S]*然后继续。/);
    }
  });

  it('leaves an unlabelled fence uncoloured', () => {
    const commit = "fix(webview): keep a steered message inside the running turn's stream\n\n- the fold's clip no longer shaves it";
    const html = renderToStaticMarkup(createElement(CodeBlock, { code: commit }));
    // Only the <code> body matters: the hover copy button beside it renders its own status span. Every line is a span
    // of its own (.code-line); a coloured token would be a span with a style
    const body = /<code[^>]*>[\s\S]*<\/code>/.exec(html)?.[0] ?? '';
    expect(body).not.toContain('style=');
    // The lines' text is the source exactly, the blank line's newline included
    expect(body.replace(/<[^>]+>/g, '').replace(/&#x27;/g, "'")).toBe(commit);
    expect(html).not.toContain('code-syntax');
    expect(html).toContain('aria-label="Copy code"');
    expect(html).toContain('turn&#x27;s stream');
  });
});

describe('fence languages', () => {
  it.each([
    ['ts', 'typescript'], ['TSX', 'tsx'], ['bash', 'shellscript'], ['shell', 'shellscript'], ['py', 'python'],
    ['rust', 'rust'], ['yml', 'yaml'], ['json', 'json'],
  ])('maps %s to %s', (tag, language) => expect(fenceLanguage(tag)).toBe(language));

  it.each([undefined, '', 'text', 'plaintext', 'constructor', 'toString', 'kotlin'])('keeps %s plain', tag =>
    expect(fenceLanguage(tag)).toBeNull());
});
