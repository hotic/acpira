import { describe, expect, it } from 'vitest';
import { looksLikeMarkdown } from '../src/webview/chat/markdownGuess';

describe('looksLikeMarkdown', () => {
  it('recognizes typical pasted documents', () => {
    const plan = [
      '> **让 X 成为一等公民**',
      '',
      '# 一、先说结论',
      '',
      '| 模块 | 当前检查结果 |',
      '| --- | --- |',
      '| A | B |',
      '',
      '详见 [OpenCode](https://example.com) 和 `service.ts`。',
      '',
      '---',
    ].join('\n');
    expect(looksLikeMarkdown(plan)).toBe(true);
    expect(looksLikeMarkdown('# 标题\n\n正文 **加粗** 再 **加粗**')).toBe(true);
    expect(looksLikeMarkdown('- one\n- two\n- three\n\n**note** and **more**')).toBe(true);
    expect(looksLikeMarkdown('> quoted line\n\n`code` and **bold** and **bold**')).toBe(true);
    expect(looksLikeMarkdown('text\n\n```ts\nconst x = 1;\n```')).toBe(true);
  });

  it('leaves code, configs and logs alone', () => {
    const python = '# file comment\ndef f(**kwargs):\n    return None  # tail\n';
    const shell = '#!/bin/sh\n# setup\necho `date` >> log\n# done\n';
    const yaml = '---\nname: x\nitems:\n  - a\n  - b\n  - c\n';
    const diff = '--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n';
    const log = '[2026-09-20 12:00] INFO start\n[2026-09-20 12:01] INFO done\n';
    const c = 'int **a; int **b;\nint **c; int **d;\n';
    for (const text of [python, shell, yaml, diff, log, c, '', 'hello world', '1. a\n2. b']) {
      expect(looksLikeMarkdown(text), JSON.stringify(text.slice(0, 40))).toBe(false);
    }
  });

  it('does not trust a lone signal that code shares', () => {
    expect(looksLikeMarkdown('# just a comment\nprint(1)')).toBe(false);
    expect(looksLikeMarkdown('a `cmd` b\nc `cmd` d')).toBe(false);
    expect(looksLikeMarkdown('**only** this is bold')).toBe(false);
  });
});
