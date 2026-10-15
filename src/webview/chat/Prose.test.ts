import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { Prose } from './Prose';
import { CodeBlock } from './CodeBlock';
import { fenceLanguage } from './codeSyntax';
import { QuietEntranceContext } from '../ui/Row';

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

  it('leaves an unlabelled fence uncoloured', () => {
    const commit = "fix(webview): keep a steered message inside the running turn's stream\n\n- the fold's clip no longer shaves it";
    const html = renderToStaticMarkup(createElement(CodeBlock, { code: commit }));
    // Only the <code> body matters: the hover copy button beside it renders its own status span
    const body = /<code[^>]*>[\s\S]*<\/code>/.exec(html)?.[0] ?? '';
    expect(body).not.toContain('<span');
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
