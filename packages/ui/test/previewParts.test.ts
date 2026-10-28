import { describe, expect, it } from 'vitest';
import { previewParts } from '../panel/previewParts';

// Every boundary taken, so a case shows where a part may start; each part is named by its first line
const heads = (markdown: string) => {
  const parts = previewParts(markdown, { whole: 0, part: 1 });
  expect(parts.join('')).toBe(markdown);
  return parts.map(part => part.split(/\r?\n/)[0]);
};

describe('Markdown preview parts', () => {
  it('starts a part at a block on the left edge after a blank line', () => {
    expect(previewParts('# One\n\nText.\n\n## Two\r\n\r\nMore\r\n', { whole: 0, part: 1 }))
      .toEqual(['# One\n\n', 'Text.\n\n', '## Two\r\n\r\n', 'More\r\n']);
    expect(previewParts(`# One\n\n${'Text. '.repeat(100)}\n\n## Two\n`)).toHaveLength(1);
  });

  it('keeps a fence, a list and a quote in one part', () => {
    expect(heads('A\n\n```js\n# x\n\ny\n```\n\nB\n')).toEqual(['A', '```js', 'B']);
    expect(heads('A\n\n````\n```\n\nz\n````\n\nB\n')).toEqual(['A', '````', 'B']);
    expect(heads('A\n\n~~~\n\n# x\n~~~\n\n$$\n\n# y\n$$\n\nB\n')).toEqual(['A', '~~~', '$$', 'B']);
    expect(heads('- a\n\n- b\n\n  c\n\n> q\n\n> r\n\n1. x\n\nEnd\n')).toEqual(['- a', 'End']);
    expect(heads('A\n\n<!--\n\nB\n-->\n\nC\n')).toEqual(['A', '<!--']);
  });

  it('waits for HTML elements the parser carries past a paragraph', () => {
    expect(heads('<p align="center">\n<img src="x">\n</p>\n\nA\n\n<details>\n\nB\n\n</details>\n\nC\n'))
      .toEqual(['<p align="center">', 'A', '<details>', 'C']);
    expect(heads('Use <b>bold\n\nStill bold\n\nEnd</b>\n\nAfter\n')).toEqual(['Use <b>bold', 'After']);
    expect(heads('A <span>x\n\nB\n')).toEqual(['A <span>x', 'B']);
    expect(heads('<picture>\n<img src="x">\n</picture>\n\nA\n')).toEqual(['<picture>', 'A']);
    // A custom element left open past its HTML block, a cut-off end tag, a table part opening a fragment
    expect(heads('A\n\n<Warning>\nText\n\nMore\n</Warning>\n\nB\n\nC\n')).toEqual(['A', '<Warning>']);
    expect(heads('<div>\n\nA\n\n</div\n\nB\n')).toEqual(['<div>']);
    expect(heads('A\n\n<caption>x</caption>\n\nB\n')).toEqual(['A', 'B']);
    // A raw-text element takes the rest as text, a fragment drops an end tag the whole made an element, and an end
    // tag in a code span is text
    expect(heads('<div>\n<iframe src="x" />\n</div>\n\nA\n')).toEqual(['<div>']);
    expect(heads('<div>\nx\n</div>\n\n</br>\n\nA\n')).toEqual(['<div>', 'A']);
    expect(heads('<details>\n\nUse `</details>` to close\n\nB\n\n</details>\n\nC\n')).toEqual(['<details>', 'C']);
    // A raw-text tag in a code span is text; any heading's end tag ends the heading open last
    expect(heads('A `<title>` B\n\n<details>\n\nC\n\n</details>\n\nD\n')).toEqual(['A `<title>` B', '<details>', 'D']);
    expect(heads('<h1>A</h2>\n\n<details><h2>B</h1>\n\nC\n\n</details>\n\nD\n')).toEqual(['<h1>A</h2>', '<details><h2>B</h1>', 'D']);
    // A fence line inside an HTML block is the block's text: the next one opens a fence, which holds C
    expect(heads('A\n\n<img src="x">\n```\n\nB\n\n```\n\nC\n')).toEqual(['A', '<img src="x">', 'B', '```']);
    // The parser moves a formatting element down into the special elements above it eight times at most
    expect(heads(`<div><b>${'<div>'.repeat(7)}x</b>${'</div>'.repeat(8)}\n\nB\n`)).toHaveLength(2);
  });

  it('follows HTML tables, list items and reference labels', () => {
    // Markdown in a cell; a formatting element opened in a table outside its cells reopens after it
    expect(heads('<table>\n<tr>\n<td>\n\n**A**\n\n</td>\n</tr>\n</table>\n\nB\n')).toEqual(['<table>', 'B']);
    expect(heads('<table><b><tr><td>A</td></tr></table>\n\nB\n\nC</b>\n\nD\n')).toEqual(['<table><b><tr><td>A</td></tr></table>', 'D']);
    // A block in a list item ends with it; a label no definition names is text, its tag a tag
    expect(heads('- <details>\n\n  A\n\nB\n')).toEqual(['- <details>', 'B']);
    expect(heads('See [x][<b>y]\n\nC\n\nD</b>\n\nE\n')).toEqual(['See [x][<b>y]', 'E']);
  });

  it('tells a lazy line from one past a list or quote', () => {
    // A line without markers carries on the quote's paragraph, so the quote's end closes what it opens; an item with
    // nothing on its marker's line holds the next line; past a blank line, indentation tells
    expect(heads('> a\nb</p><l>\n\nC\n')).toEqual(['> a', 'C']);
    expect(heads('*\n  <l>\n\nC\n')).toEqual(['*', 'C']);
    expect(heads('   9. a\n      b\n\n   END\n\nC\n')).toEqual(['   9. a', 'C']);
  });

  it('gives each part the link definitions it names, the first of a label first', () => {
    // Definitions alone render nothing, so they go with the part after them, or at the end with the one before
    expect(previewParts('[a]: /one\n\nSee [a].\n\nMiddle\n\nSee [A] again.\n\n[a]: /two\n[b]: /three "Three"\n', { whole: 0, part: 1 })).toEqual([
      '[a]: /one\n[a]: /two\n\n[a]: /one\n\nSee [a].\n\n',
      'Middle\n\n',
      '[a]: /one\n[a]: /two\n[b]: /three "Three"\n\nSee [A] again.\n\n[a]: /two\n[b]: /three "Three"\n',
    ]);
    expect(heads('A\n\n<!-- x -->\n\nB\n')).toEqual(['A', '<!-- x -->']);
    // A label's words count with and without what reads as a marker at a line's start; a byte order mark stays first
    expect(previewParts('[a -b]: /u\n\nA\n\n[a\n-b]\n', { whole: 0, part: 1 }))
      .toEqual(['[a -b]: /u\n\n[a -b]: /u\n\nA\n\n', '[a -b]: /u\n\n[a\n-b]\n']);
    expect(previewParts('﻿[a]: /u\n\nB\n\n[a]\n', { whole: 0, part: 1 }))
      .toEqual(['﻿[a]: /u\n\n[a]: /u\n\nB\n\n', '[a]: /u\n\n[a]\n']);
  });

  it('splits above a definition it cannot place when no part above may name it', () => {
    // A word past the destination, or past a title closed on its line, makes none, so the line is text
    expect(heads('A\n\n[a]: /u x\n\nB [a]\n')).toEqual(['A', '[a]: /u x', 'B [a]']);
    expect(heads('A\n\n[a]: /u "t" x\n\nB [a]\n')).toEqual(['A', '[a]: /u "t" x', 'B [a]']);
    // One in a list item stops the scan, but the parts above hold none of its label's words, or no footnote mark
    expect(heads('One\n\nTwo\n\n- [a]: /u\n\nSee [a].\n')).toEqual(['One', 'Two']);
    expect(heads('One\n\nTwo\n\n[^1]: Note\n\nC[^1]\n')).toEqual(['One', 'Two']);
  });

  it('leaves whole what cannot be told apart from a line scan', () => {
    for (const markdown of [
      // Definitions in a list, after paragraph text, with a title on the next line or unbalanced parentheses, and a
      // footnote
      '- [F]: Fix\n\nB\n',
      'A\n[x]: /x\n\nB [x]\n',
      'A\n\n[x]: /x\n  "Title"\n\nB [x]\n',
      'A\n\n[x]: /x)(\n\nB [x]\n',
      'A[^1]\n\n[^1]: Note\n\nB\n',
      // A blank label makes no definition, so its line is text, which a task item's `[ ]` must not pull in
      'A\n\n[ ]: /x\n\n- [ ] todo\n',
      // A fence in a list item that a line on the left edge ended: the next fence line opens one, which holds C
      '- a\n  ```\n  x\n\nB\n\n  ```\n\nC\n',
      // A list in a list item takes the item's end tag; a heading ends a list, so the tag below it opens at the top
      '- <ul>\n\nB\n\nC\n',
      '- a\n# H\n  <span>\n\nB\n',
      // After indented code (past blank lines too) or a definition, a list numbered past 1 is a paragraph's text
      '    code\n\n2. <details>\n\nB\n',
      '[a]: /u\n2. <details>\n\nB\n',
      // A delimiter row makes a definition the table's header, and a tag alone on its line over one an HTML block; a
      // table's body ends at such a tag and takes rows without a bar
      '[a]: /u\n|-\n\nB\n',
      '[a]: /u\n:-\n\nB\n',
      'A\n<span>\n|-\n\nB\n',
      '| a |\n|---|\n<span>\n\nB\n',
      '| a |\n|---|\nx </table>\ny\n\nB\n',
      // A one-cell delimiter row needs only a colon, and a body row no bar, in a list item too; a tag in a cell reaches
      // nothing from before its line, even what the paragraph the table breaks into left open; a setext underline in a
      // table is a row, so the line below is no definition
      '<b>\n\na\n:-:\nx </b>\n\nB\n',
      '<b>\n\n- a\n  :-:\n  x </b>\n\nB\n',
      'x <a>\n| a |\n|-|\n| </a> |\n\nB\n',
      'x\n-:\n=\n[a]: /u\n\nB [a]\n',
      // The same over CRLF line ends: the line below is read without its CR
      '<b>\r\n\r\nx </b>\r\n:-:\r\n\r\nB\r\n',
      // An end tag that is no tag: in an attribute value, a comment, after an escape, in inline math
      '<b title="</b>">A\n\nB\n',
      '<b><!-- </b> -->A\n\nB\n',
      '<b>A \\</b>\n\nB\n',
      '<details>\n\nA $$x </details>$$\n\nB\n\n</details>\n',
      // Elements the parser closes early or not at all: a paragraph by `hr`, only the form by `</form>`
      'A <hr><del>B\n\nC</del>\n\nD\n',
      '<form>\n<details>\n</form>\n\nA\n\n</details>\n',
      // A `pre` block over a blank line holds what looks like a definition; labels over two lines or with an escape
      '<pre>\nA\n\n[x]: /x\n</pre>\n\nB [x]\n',
      '[a\nb]: /x\n\nA\n\n[a b]\n',
      'A\n===\n[a\nb]: /x\n\nB\n\n[a b]\n',
      '[a\\]b]: /x\n\nA\n\n[a\\]b]\n',
      // A line without markers after a break, a fence or a blank line in a list or quote stands past it, and so does one
      // under an item that a blank line after its marker left empty: what it opens stays open
      '>---\na</p><l>\n\nC\n',
      '- a\n  ```\n  x\n  ```\nb</p><l>\n\nC\n',
      '- a\n\n b</p><l>\n\nC\n',
      '*\n\n  <l>\n\nC\n',
      // A definition the scan cannot place that a part above may name: by its label's words, read past what may be
      // markers too, or by any footnote mark
      'See [a].\n\nTwo\n\n- [a]: /u\n',
      'See [a b].\n\nC\n\n> [a\n> b]: /u\n',
      '[api 1.2.3]\n\n]\n>[api\n>1.2.3]:>',
      'A[^1]\n\nB\n\n- [^1]: Note\n',
      // A byte order mark first, which the parser drops, or past it; a table opened in emphasis; a formatting element
      // under eight special ones; an unquoted attribute value, which a slash ends
      '﻿```\n\nB\n',
      'A\n\n﻿# B\n',
      '**<table><tr><td>x**\n\n</table>\n\nB\n',
      `<div><b>${'<div>'.repeat(8)}x</b>${'</div>'.repeat(9)}\n\nB\n`,
      'X <img src=https://example.com/x title="<b>">\n\nB\n',
      // The adoption agency drops an ordinary element in between, and keeps three alike formatting elements at most
      '<div></div><b><del><div></b></div><ins></del>\n\nB\n',
      '<div></div><b><b><b><b><div></b></b></b></b></div>\n\nB\n',
      // Strikethrough around a tag once a tag closed the paragraph, or in a heading's line, where it keeps a heading's
      // start from closing the heading
      'x <div></div> ~~<div>~~ </div>\n\nB\n',
      '# ~~<h2>~~\n\nC\n',
      // An `a` closes an `a` still open, and so does a Markdown link; a section's end tag in a cell does nothing
      '<h1><a><a></a><h2></h2></a><details></h1>\n\nInside\n\n</details>\n',
      'x <h1><a>[x](u)<h2></h2></a><details></h1>\n\nInside\n\n</details>\n',
      '<h1><table><td></thead><b></table><h2></h2></b><details></h1>\n\nInside\n\n</details>\n',
      // A bare URL takes in a backtick, which then opens no code span
      'See https://example.com/` <details> `\n\nInside\n\n</details>\n',
    ]) expect(heads(markdown)).toHaveLength(1);
  });
});
