import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { Prose } from './Prose';
import { CodeBlock } from './CodeBlock';
import { fenceLanguage } from './codeSyntax';

const prose = (markdown: string) => renderToStaticMarkup(createElement(Prose, { block: { type: 'text', markdown } as never }));

describe('markdown prose', () => {
  it('closes bold runs that end on CJK punctuation directly before a CJK letter', () => {
    // CommonMark alone leaves `**…。**范围` literal: the closer is preceded by punctuation and followed by a letter
    const html = prose('老板，**以后不会再卡住；但没有改成免 Passkey。**范围得说清楚');
    // streamdown renders strong as a marked span
    expect(html).toContain('data-streamdown="strong">以后不会再卡住；但没有改成免 Passkey。</span>范围');
    expect(html).not.toContain('**');
  });

  it('leaves an unlabelled fence uncoloured', () => {
    const commit = "fix(webview): keep a steered message inside the running turn's stream\n\n- the fold's clip no longer shaves it";
    const html = renderToStaticMarkup(createElement(CodeBlock, { code: commit }));
    expect(html).not.toContain('<span');
    expect(html).not.toContain('code-syntax');
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
