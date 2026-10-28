// A long Markdown document previews in parts, mounted a frame apart from the top (FileViewer `MarkdownPreview`), so the
// first screen shows at once: rendered whole, a 160 KB file of 600 sections held one 0.7 s frame. A part starts only
// where the whole would have closed everything: at a line on the left edge after a blank line that no list or quote
// takes in, outside a fence or comment, with no HTML element open. A line scan finds those for a fraction of a parse
// (micromark's tokens alone cost about as much as the whole render). It follows the HTML parser over the elements of
// prose and plain blocks, with the paragraph or heading Markdown wraps around a line, and stops splitting at anything
// else: the rest of the document is then one part. Where it cannot tell a tag from text, a start tag counts (as an
// element that may be none) and an end tag does not; either one stops the splitting if it would close an element, and so
// does a sure tag whose effect an element that may be none would change. Link reference definitions resolve
// across the document, so each part gets the ones it may name in front (they render nothing, and the first of a label
// still wins); a definition the scan cannot place for sure, or a footnote, leaves the document whole
const PREVIEW_WHOLE_MAX = 24 * 1024;
const PREVIEW_PART = 4 * 1024;
// What micromark reads otherwise than the scan (a bare CR or another control character, a line or paragraph
// separator), or what the scan does not follow anywhere: `plaintext`, task notes
const SPLIT_UNSAFE = /[\0-\x08\x0b\x0c\x0e-\x1f\x7f\u2028\u2029]|\r(?!\n)|[^]\ufeff|<task-notification|<plaintext(?![^\t\n />])/i;
// A processing instruction, CDATA or a declaration: Markdown ends the first two later than the HTML parser
const MARKUP = /^<(?:\?|![a-z[])/i;
// A code or math fence up to three columns in; a deeper one sits in a list item or is code
const FENCE = /^( {0,3})(`{3,}|~{3,}|\${2,})(.*)$/;
// A line on the left edge that a list or a quote above may still take in
const TAKEN_IN = /^(?:>|[-+*](?:[ \t]|$)|\d{1,9}[.)](?:[ \t]|$))/;
// Quote and list markers in front of a line's content, and the indentation after them
const MARKERS = /^(?:[ \t]*(?:>|[-+*](?=[ \t]|$)|\d{1,9}[.)](?=[ \t]|$)))*/;
// Blocks that end a paragraph above them, besides fences and HTML blocks: a heading, a quote, a list item that may start
// there (a bullet or `1.` with content), a thematic break, or a setext underline (which makes the paragraph a heading)
const ATX = /^ {0,3}#{1,6}(?:[ \t]|$)/;
const INTERRUPTS = /^ {0,3}(?:>|[-+*][ \t]+[^ \t]|1[.)][ \t]+[^ \t])/;
const BREAK = /^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$/;
const SETEXT = /^ {0,3}(?:=+|-+)[ \t]*$/;
// A bar that may split a table row into cells, and what may be a table's delimiter row: a one-cell one needs no bar,
// only a colon (`:-`)
const PIPE = /(?:^|[^\\])(?:\\\\)*\|/;
const delimiterRow = (line: string) => line.includes('-') && /[|:]/.test(line) && /^[ \t|:-]+$/.test(line);
// A part's HTML is parsed as a fragment, whose first tag sets the mode: a table part switches it to table parsing, and
// an end tag is dropped where the whole would have made `</p>` or `</br>` an element
const FRAGMENT_START = /^<(?:\/|(?:caption|col|colgroup|tbody|tfoot|thead|tr|td|th)(?![^\t />]))/i;
// Anything that may be a link reference or footnote definition, in a list or quote too, escapes and all; on several
// lines, a label running over them
const MAYBE_DEFINITION = /^[ \t]*(?:(?:[-+*>]|\d{1,9}[.)])[ \t]*)*\[(?:[^[\]\\]|\\[\s\S])*\]:/m;
const MAYBE_DEFINITIONS = /^[ \t]*(?:(?:[-+*>]|\d{1,9}[.)])[ \t]*)*\[((?:[^[\]\\]|\\[\s\S])*)\]:/gm;
// What surely is none: past a label (not a footnote's) and its colon, a destination (not in angle brackets, which may
// hold spaces) and then, on its line past spaces, what cannot open a title, or a title closed on the line and more
const NO_DEFINITION = /^[ \t]*(?:(?:[-+*>]|\d{1,9}[.)])[ \t]*)*\[(?!\^)(?:[^[\]\\]|\\[\s\S])*\]:[ \t]*[^\s<]\S*[ \t]+(?:[^\s"'(]|(?:"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|\((?:[^()\\]|\\.)*\))[ \t]*\S)/;
// A label still open at the end of its line, which a line below may close
const LABEL_OPEN = /^[ \t]*(?:(?:[-+*>]|\d{1,9}[.)])[ \t]*)*\[(?:[^\]\\]|\\.)*\\?$/;
// A definition on one line, of the plain shapes: a label without brackets, escapes or a leading `^`, a destination in
// angle brackets or without spaces, an optional title
const DEFINITION = /^ {0,3}\[([^[\]\\\n^][^[\]\\\n]{0,998})\]:[ \t]*(<[^<>\n]*>|[^\s<]\S*)(?:[ \t]+(?:"[^"\n]*"|'[^'\n]*'|\([^()\n]*\)))?[ \t]*$/;
// HTML blocks that start whatever came before, even a paragraph: a `pre`, `script`, `style` or `textarea` one, which
// runs over blank lines to a line holding one of their end tags, and one of the block names. Another tag alone on its
// line starts one too, but cannot break into a paragraph
const RAW_BLOCK = /^<(?:pre|script|style|textarea)(?![^\t >])/i;
const RAW_BLOCK_END = /<\/(?:pre|script|style|textarea)>/i;
const BLOCK_NAMES = 'address article aside base basefont blockquote body caption center col colgroup dd details dialog dir div dl dt '
  + 'fieldset figcaption figure footer form frame frameset h[1-6] head header hr html iframe legend li link main menu menuitem nav '
  + 'noframes ol optgroup option p param search section summary table tbody td tfoot th thead title tr track ul';
const BLOCK_START = new RegExp(`^</?(?:${BLOCK_NAMES.split(' ').join('|')})(?:[\\t >]|\\/>|$)`, 'i');
// Tags as micromark reads them in Markdown: a whole start tag, attributes and all, on one line (an unquoted value ends
// at a slash, which must close the tag); an end tag
const WHOLE_TAG = /<[a-z][a-z0-9-]*(?:[ \t]+[a-z_:][\w.:-]*(?:[ \t]*=[ \t]*(?:[^ \t"'=<>`/]+|'[^']*'|"[^"]*"))?)*[ \t]*\/?>/iy;
const END_TAG = /<\/([a-z][a-z0-9-]*)[ \t]*>/iy;
const START_NAME = /<([a-z][a-z0-9-]*)(?![^\t />])/iy;
const LONE_TAG = /^(?:<[a-z][a-z0-9-]*(?:[ \t]+[a-z_:][\w.:-]*(?:[ \t]*=[ \t]*(?:[^ \t"'=<>`/]+|'[^']*'|"[^"]*"))?)*[ \t]*\/?>|<\/[a-z][a-z0-9-]*[ \t]*>)[ \t]*$/i;
// Elements the scan does not follow: the parser ignores them in a body, switches its mode or tokenizer for them, keeps
// state off its stack for them (the form pointer), or closes them by rules the scan lacks. One stops the splitting
const UNFOLLOWED = new Set(('applet body button dialog form frame frameset head html iframe isindex listing marquee math '
  + 'menuitem noembed noframes noscript object optgroup option plaintext rb rp rt rtc script search select style svg template '
  + 'textarea title xmp').split(' '));
// A table's parts, followed in HTML outside lists and quotes: the cells (`td`, `th`, `caption`) stand on the stack, the
// rows and sections only end cells. What opens in a cell ends with it; what opens in the table outside its cells
// moves out before it, and a formatting element among those reopens after it
const TABLE_PARTS = new Set('caption col colgroup tbody td tfoot th thead tr'.split(' '));
const CELL = new Set(['td', 'th', 'caption']);
// The parser's scope searches stop at a table and its cells
const SCOPE = new Set(['table', 'td', 'th', 'caption']);
// How the HTML parser ends the rest. A formatting element only by its own end tag: an enclosing element's end reopens
// it in what follows
const FORMATTING = new Set('a b big code em font i nobr s small strike strong tt u'.split(' '));
// Void ones, the head ones a body takes in and drops, and `image` (read as `img`)
const VOID = new Set('area base basefont bgsound br embed hr image img input keygen link meta param source track wbr'.split(' '));
// The special ones among those followed: another element's end tag gives up at one, and `li`, `dd` and `dt` stop looking
// for one of their kind to close at one but `address`, `div` and `p`
const SPECIAL = new Set(('address area article aside base basefont bgsound blockquote br caption center col colgroup dd '
  + 'details dir div dl dt embed fieldset figcaption figure footer h1 h2 h3 h4 h5 h6 header hgroup hr img input keygen li link '
  + 'main menu meta nav ol p param pre section source summary table tbody td tfoot th thead tr track ul wbr').split(' '));
// Start tags that close a paragraph open above them (`table` too: the parser is not in quirks mode), the blocks
const CLOSES_P = new Set(('address article aside blockquote center dd details dir div dl dt fieldset figcaption figure footer '
  + 'h1 h2 h3 h4 h5 h6 header hgroup hr li main menu nav ol p pre section summary table ul').split(' '));
// Start tags past what the scan follows in a list or quote
const IN_CONTAINER_STOPS = new Set('blockquote dd dt li ol pre ul'.split(' '));
// End tags that end a list's or quote's own elements, from inside one
const CONTAINER_TAGS = new Set('blockquote dd dl dt li ol ul'.split(' '));
const HEADING = /^h[1-6]$/;
// The head elements followed: first in a part they leave the parser where it starts, so a `</p>` or `</br>` after them is
// dropped where the whole made an element of it
const HEAD = new Set('base basefont bgsound link meta'.split(' '));
// Quote markers alone, each a quote's (not code in the one around it)
const QUOTES = /^ {0,3}>(?: {0,4}>)*$/;
// A line that may start an HTML block
const htmlStart = (head: string) => RAW_BLOCK.test(head) || BLOCK_START.test(head) || LONE_TAG.test(head) || head.startsWith('<!--');
// Inline Markdown that may wrap a tag in an element of its own: emphasis, strikethrough, a link's text
const MARKS = /[*_~[]/;
// Inline Markdown that makes a link: a reference or resource, an autolink, a bare URL or address
const LINKISH = /\[|<[a-z][a-z0-9+.-]{1,31}:|<[^\s<>@]+@|:\/\/|www\.|\w@\w/i;
// A bare URL with a backtick or dollar in it, which then is no span's mark
const URL_TICK = /(?:https?:\/\/|www\.)[^\s<]*[`$]/i;
// A label as matching compares it: micromark's `normalizeIdentifier`
const label = (text: string) => text.replace(/[\t\n\r ]+/g, ' ').replace(/^ | $/g, '').toLowerCase().toUpperCase();
// A label word as it may read past quote and list markers that a line of it may start with (a label runs over them,
// and its own text may look like one: `1.2.3`): the word, then without each marker in turn
const readings = (word: string) => {
  const out = [word];
  for (let m; (m = /^(?:>|[-+*]|\d{1,9}[.)])/.exec(word));) out.push(word = word.slice(m[0].length));
  return out;
};
// The label words a text may name, in every reading
const wordsOf = (text: string) => new Set(label(text).split(/[\t\n\r [\]]+/).flatMap(readings));

// Where the HTML tokenizer stands inside a tag: in its name, between attributes, in a name, after one, before a value,
// in a quoted or unquoted value, after a quoted one, or after a `/`
type TagState = 'name' | 'before' | 'attr' | 'after' | 'value' | 'quoted' | 'unquoted' | 'valueEnd' | 'slash';
interface InTag { state: TagState; quote: string; sets: boolean }

// Runs the HTML tokenizer through a tag from `i`: the index past its `>`, or -1 at the line's end, with `at` left where
// the line ending puts it (a line ending is a space, but inside a quoted value)
function tagEnd(line: string, i: number, at: InTag): number {
  for (; i <= line.length; i++) {
    const c = i < line.length ? line[i]! : '\n';
    const space = c === ' ' || c === '\t' || c === '\n';
    switch (at.state) {
      case 'quoted':
        if (c === at.quote) at.state = 'valueEnd';
        break;
      case 'unquoted':
        if (space) at.state = 'before';
        else if (c === '>') return i + 1;
        break;
      case 'value':
        if (c === '"' || c === "'") { at.state = 'quoted'; at.quote = c; }
        else if (c === '>') return i + 1;
        else if (!space) at.state = 'unquoted';
        break;
      case 'name':
      case 'attr':
        if (space) at.state = at.state === 'name' ? 'before' : 'after';
        else if (c === '/') at.state = 'slash';
        else if (c === '>') return i + 1;
        else if (c === '=' && at.state === 'attr') at.state = 'value';
        break;
      default:
        // Between attributes, after a name or a quoted value, after a `/`: a `/` or `>` acts, a `=` after a name starts
        // its value, and anything else starts a name (a `=` too, where no name came before)
        if (c === '>') return i + 1;
        if (c === '/') at.state = 'slash';
        else if (c === '=' && at.state === 'after') at.state = 'value';
        else if (space) at.state = at.state === 'after' ? 'after' : 'before';
        else at.state = 'attr';
    }
  }
  return -1;
}

// The column of a line's first character other than a space or tab (a tab runs to the next multiple of four), -1 if none
function columnOf(line: string): number {
  let col = 0;
  for (const c of line) {
    if (c === ' ') col++;
    else if (c === '\t') col += 4 - (col % 4);
    else return col;
  }
  return -1;
}

// The column a list item's content starts at, from its marker line, or -1 for a quote. Never left of where Markdown puts
// it: a wider gap after the marker (tabs included) only moves it right
function itemOffset(line: string): number {
  const item = /^[ \t]*([-+*]|\d{1,9}[.)])([ \t]*)/.exec(line);
  if (!item) return -1;
  const marker = columnOf(line) + item[1]!.length;
  let col = marker;
  for (const c of item[2]!) col += c === '\t' ? 4 - (col % 4) : 1;
  return col - marker >= 5 || item[0].length === line.length ? marker + 1 : col;
}

// Where a line holds what looks like a tag but may be none: a link's destination and title, an image's alt, the label of
// a reference or footnote call that `defined` may hold (resolved, it renders nothing). `carry`: one runs on past the
// line's end, on to the paragraph's next line
function linkRegions(line: string, defined: Set<string>): { regions: [number, number][]; carry: boolean } {
  const regions: [number, number][] = [];
  let carry = false;
  const esc = (i: number) => { let k = i; while (k > 0 && line[k - 1] === '\\') k--; return (i - k) % 2 === 1; };
  for (const m of line.matchAll(/\]\(|!\[|\]\[|\[\^/g)) {
    const from = m.index;
    let i = from + 2;
    if (m[0] === '](') {
      // A resource: a destination, a title, then `)`, any of them maybe on the next line
      while (line[i] === ' ' || line[i] === '\t') i++;
      if (line[i] === '<') {
        while (++i < line.length && (line[i] !== '>' || esc(i)) && line[i] !== '<');
        if (line[i] !== '>') { regions.push([from, line.length]); continue; }
        i++;
      } else {
        for (let depth = 0; i < line.length && line[i] !== ' ' && line[i] !== '\t'; i++) {
          if (esc(i)) continue;
          if (line[i] === '(') depth++;
          else if (line[i] === ')' && --depth < 0) break;
        }
      }
      while (line[i] === ' ' || line[i] === '\t') i++;
      const quote = line[i] === '"' || line[i] === "'" ? line[i] : line[i] === '(' ? ')' : '';
      if (quote) {
        while (++i < line.length && (line[i] !== quote || esc(i)));
        if (i >= line.length) { regions.push([from, line.length]); carry = true; continue; }
        i++;
        while (line[i] === ' ' || line[i] === '\t') i++;
      }
      if (i >= line.length) carry = true;
      regions.push([from, line[i] === ')' ? i + 1 : line.length]);
    } else {
      // An alt or a label, up to its `]`
      if (m[0] === '[^') i--;
      const text = i;
      for (let depth = 0; i < line.length; i++) {
        if (esc(i)) continue;
        // In an alt, which reads as a link's text, a tag, an autolink or a code span may hold a `]` that ends nothing:
        // past telling
        if (m[0] === '![' && (line[i] === '<' || line[i] === '`')) i = line.length;
        else if (line[i] === '[') depth++;
        else if (line[i] === ']' && --depth < 0) break;
      }
      if (i >= line.length) carry = true;
      // A label no definition names is text, or another label's link text
      else if (m[0] !== '![' && !defined.has(label(line.slice(text, i)))) continue;
      regions.push([from, Math.min(i + 1, line.length)]);
    }
  }
  return { regions, carry };
}

// The HTML parser ends a comment at `--!>` too, where Markdown does not: one in it from `from` up to `end` (-1: the
// line's end)
function bang(line: string, from: number, end: number): boolean {
  const at = line.indexOf('--!>', from);
  return at >= 0 && (end < 0 || at < end);
}

// A destination without angle brackets may hold parentheses only in balanced pairs
function balanced(destination: string): boolean {
  let depth = 0;
  for (let i = 0; i < destination.length; i++) {
    const c = destination[i];
    if (c === '\\') i++;
    else if (c === '(') depth++;
    else if (c === ')' && --depth < 0) return false;
  }
  return depth === 0;
}

// The parts join back to `markdown`, but for the definitions in front of them. `whole`: the longest document left
// whole; `part`: the shortest part
export function previewParts(markdown: string, { whole = PREVIEW_WHOLE_MAX, part = PREVIEW_PART } = {}): string[] {
  if (markdown.length <= whole || SPLIT_UNSAFE.test(markdown)) return [markdown];
  const parts: string[] = [];
  const definitions: { words: string[]; line: string }[] = [];
  // Every label a definition or footnote may give, wherever it stands
  const defined = new Set<string>();
  for (const m of markdown.matchAll(/\[((?:[^[\]\\]|\\[^])+)\]:/g)) defined.add(label(m[1]!));
  // Elements open at this point, innermost last: those of the HTML, and as `p` or `h1` the paragraph or heading
  // Markdown wraps around the line
  const open: string[] = [];
  // Whether each of those may be none: one a tag that may be text opened. Where the scan's next step would differ
  // without it, the scan stops
  const doubt: boolean[] = [];
  let start = 0;
  let fence: { mark: string; size: number; indent: number } | undefined;
  // An HTML comment block, which runs to the line holding its end, blank lines and all
  let comment: { indent: number } | undefined;
  // What the line before was: a definition may only follow one that ended a block, and a tag alone on its line after
  // text carries the paragraph on
  let before: 'blank' | 'end' | 'definition' | 'text' = 'blank';
  // An indented code block may go on past the blank lines since its last line: a list starts there only where it may
  // break into a paragraph
  let codeOpen = false;
  // In an HTML block, which runs to a blank line; a `pre` one runs to its end tag's line
  let html = false;
  let rawBlock = false;
  // A tag or comment of an HTML block that runs past its line
  let inTag: InTag | undefined;
  let inComment = false;
  // A paragraph goes on: where its `p` started on the stack, whether it held a tag (a setext underline would make it a
  // heading, which the scan does not follow), whether a link, image or reference in it may hold what looks like a tag,
  // whether a Markdown tag may run on from the line above, and whether a line of it had a bar (a table may have started
  // there, whose rows need none)
  let para = false;
  let paraBase = 0;
  // The line being scanned
  let scanned = 0;
  let paraTags = false;
  let murky = false;
  let tagRuns = false;
  // A line of the paragraph held what may open an emphasis, strikethrough or link
  let marks = false;
  let tableish = false;
  // In a run of list or quote lines, from where the stack stood before it
  let container = false;
  let containerBase = 0;
  // A fence in a list item or quote that FENCE does not take (behind markers, or four columns in): its lines are code up
  // to its end. In a list one stands at `col` or right of it, and an end up to `limit`; in a quote one repeats the
  // `quotes` quote markers. A line that does neither may have ended the item or quote, and the fence with it
  let quotedFence: { mark: string; size: number; col: number; limit: number; quotes: number } | undefined;
  // Where a fence or HTML block in a list or quote ends is past telling, up to its end: what may be code is read as
  // maybe tags, and no part ends there (a fence an item ends may take in the blank lines before)
  let unsure = false;
  // The backtick or dollar run that opened a code or math span not closed yet, which a blank line ends
  let tick = '';
  // The part so far renders something: one of definitions and comments alone would stand as an empty block, a gap
  // the whole does not have, so it goes with the next
  let content = false;
  // No element has set the HTML parser's mode in the part yet: a part's HTML is parsed as a fragment, which takes its
  // mode from the first element, past comments, text and the head elements followed. A table one or an end tag there
  // reads otherwise than in the whole, so the part joins the one above
  let pristine = true;
  // Where the outermost list item's content starts (-1: a quote): a line left of it without markers may be outside the
  // list or quote. And an HTML block in an item, raw to the HTML parser up to a blank line
  let outer = -1;
  let containerHtml = false;
  // The line is surely in the list or quote: no line since the last item or quote line may have been a lazy one, which
  // follows a list or quote it ended unless it carries on a paragraph in it
  let inItem = false;
  let lazySeen = false;
  // The last line in the list or quote surely left a paragraph open there, which a line without markers carries on as a
  // lazy one. After anything else (a heading, break, fence, table, code, HTML block or nothing) such a line may stand
  // past the list or quote, where what it opens stays open
  let lazyOk = false;
  // A table's delimiter row stood in the list or quote since the last blank line: the table takes no lazy line
  let containerTable = false;
  // The outermost item has nothing on its marker's line: it holds the next line only if that is not blank
  let emptyItem = false;
  // The last thing at the top level is inline, text or an inline element of an HTML block: inline content right after it
  // shares its line in the whole (text the parser moves out of a table too), so a part may start there only with a block
  let rootInline = false;
  const lines = markdown.split('\n');
  // A line without its CR, as the scan reads it (a CRLF file's next line too), and the first without the byte order mark
  // the Markdown parser drops
  const lineAt = (k: number) => (k ? lines[k] ?? '' : lines[0]!.replace(/^\ufeff/, '')).replace(/\r$/, '');

  const push = (e: string, maybe = false) => { open.push(e); doubt.push(maybe); };
  // Of the elements from `from` up, keeps those `keep` takes
  const retain = (from: number, keep: (e: string, k: number) => boolean) => {
    const kept: number[] = [];
    for (let k = from; k < open.length; k++) if (keep(open[k]!, k)) kept.push(k);
    const elements = kept.map(k => open[k]!);
    const doubts = kept.map(k => doubt[k]!);
    open.length = doubt.length = from;
    open.push(...elements);
    doubt.push(...doubts);
  };
  // Pops the elements from `k` up, but the formatting ones: the parser reopens those in what follows
  const closeFrom = (k: number) => retain(k, (e, j) => j > k && FORMATTING.has(e));
  // A formatting element may stand open: one a block's end left to reopen inside what follows
  const reopens = () => open.some(e => FORMATTING.has(e));
  // The nearest of `name` the parser's scope searches reach, from the top (-1: none)
  const inScope = (test: (e: string) => boolean) => {
    for (let k = open.length - 1; k >= 0; k--) {
      if (test(open[k]!)) return k;
      if (SCOPE.has(open[k]!)) return -1;
    }
    return -1;
  };
  const closeP = () => {
    const k = inScope(e => e === 'p');
    if (k >= 0) closeFrom(k);
  };
  // The table or cell the stack stands in (-1: none)
  const context = () => {
    let k = open.length - 1;
    while (k >= 0 && !SCOPE.has(open[k]!)) k--;
    return k;
  };
  // A table's end: what opened in its cells goes, a formatting element opened in it outside them reopens after it
  const closeTable = (k: number) => {
    const cell = open.findIndex((e, j) => j > k && CELL.has(e));
    retain(k, (e, j) => j > k && (cell < 0 || j < cell) && FORMATTING.has(e));
  };
  // Back to the table from what stands above it outside a cell, formatting elements kept to reopen
  const clearToTable = (k: number) => retain(k + 1, e => FORMATTING.has(e));
  // The heading a heading's start pops, the parser's current node below `top` once a paragraph is closed: its index, -1
  // if that is none, undefined if past telling. A formatting element above one may stand on the stack or only wait to
  // reopen (`above`: one does), and an element in doubt may be none
  const currentHeading = (top: number, above: boolean): number | undefined => {
    for (let k = top - 1; k >= 0; k--) {
      const e = open[k]!;
      if (doubt[k] && HEADING.test(e)) return undefined;
      if (doubt[k] || FORMATTING.has(e)) above = true;
      else return HEADING.test(e) ? (above ? undefined : k) : -1;
    }
    return -1;
  };
  // A Markdown paragraph's end is its `</p>`
  const endParagraph = () => {
    if (para) closeP();
    para = paraTags = murky = tagRuns = tableish = marks = false;
    tick = '';
  };
  const nestedIn = (col: number) => outer >= 0 && col >= outer;
  // Whether what may be a definition from line `from` on, which the scan did not place, may give a link to a part above:
  // all its label's words stand there, or for a footnote's, any footnote mark (they number in document order)
  const namedAbove = (from: number) => {
    const tail = lines.slice(from).join('\n');
    const above = markdown.slice(0, start);
    let words: Set<string> | undefined;
    for (const m of tail.matchAll(MAYBE_DEFINITIONS)) {
      const end = tail.indexOf('\n', m.index + m[0].length);
      if (NO_DEFINITION.test(tail.slice(m.index, end < 0 ? undefined : end))) continue;
      const text = m[1]!;
      if (text.includes('\\') || (text.startsWith('^') && above.includes('[^'))) return true;
      words ??= wordsOf(above);
      if (label(text).split(' ').every(w => readings(w).some(v => !v || words!.has(v)))) return true;
    }
    return false;
  };
  // The rest of the document as one part, from a point where everything was closed: it holds every reference below a
  // definition in it, but one the parts above may name keeps the document whole. What stops the scan before the part's
  // first element may read otherwise there (a `script` block first is the head's), so that part goes with the one above
  const rest = () => {
    if (pristine && parts.length) start -= parts.pop()!.length;
    if (!parts.length || namedAbove(scanned)) return [markdown];
    return [...parts, markdown.slice(start)].map(withDefinitions);
  };

  // A start tag; `sure` that it is one. In a list or quote only the elements of prose are followed, which their item
  // or quote ends (the formatting ones reopen after it). `floor`: the stack up to there is out of the line's reach if it
  // is a table row, but not if it is a paragraph's
  const startTag = (t: string, sure: boolean, inContainer: boolean, floor = -1): boolean => {
    if (UNFOLLOWED.has(t)) return false;
    if (t === 'table' || TABLE_PARTS.has(t)) {
      // Followed only for sure, outside lists, quotes and what may be a Markdown table row
      if (inContainer || floor >= 0 || !sure) return false;
      const c = context();
      if (t === 'table') {
        // A table in a table outside its cells ends the first: past what the scan follows
        if (c >= 0 && open[c] === 'table') return false;
        closeP();
        push('table');
        return true;
      }
      // Outside a table the parser drops it; inside, it ends a cell open, or what stands above the table
      if (c < 0) return true;
      if (CELL.has(open[c]!)) retain(c, () => false);
      else clearToTable(c);
      if (CELL.has(t)) push(t);
      return true;
    }
    if (inContainer && !FORMATTING.has(t)) {
      // In a list or quote the item or quote ends what opens in it, but a list or quote, which would take its end tag,
      // `li`, `dd` and `dt`, which close an item, and `pre`, an HTML block that goes on over blank lines. A block on a
      // line that may stand past the list or quote is past what the scan follows
      return inItem ? !IN_CONTAINER_STOPS.has(t) : !SPECIAL.has(t) || VOID.has(t);
    }
    // `li`, `dd` and `dt` close one of their kind; a block closes a paragraph; a heading closes one just opened
    let item = -1;
    if (t === 'li' || t === 'dd' || t === 'dt') {
      for (let k = open.length - 1; k >= 0; k--) {
        const e = open[k]!;
        const found = e === t || (t !== 'li' && (e === 'dd' || e === 'dt'));
        const stops = SPECIAL.has(e) && e !== 'address' && e !== 'div' && e !== 'p';
        if (doubt[k] && (found || stops)) return false;
        if (found) { item = k; break; }
        if (stops) break;
      }
    }
    // A paragraph that may not be one would leave what it would close past telling
    if (t === 'p' && !sure) return false;
    const p = CLOSES_P.has(t) ? inScope(e => e === 'p') : -1;
    // A heading closes one only when that is the parser's current node, which a formatting element it reopens may
    // become, and which in a table's cell is never one from before the line
    const current = HEADING.test(t) ? currentHeading(p >= 0 ? p : open.length, p >= 0 && open.some((e, j) => j > p && FORMATTING.has(e))) : -1;
    if (current === undefined || (current >= 0 && (reopens() || current <= floor))) return false;
    if (!sure && (item >= 0 || p >= 0 || current >= 0)) return false;
    if ((item >= 0 && item <= floor) || (p >= 0 && p <= floor)) return false;
    if (item >= 0) closeFrom(item);
    if (CLOSES_P.has(t)) closeP();
    if (current >= 0) retain(current, () => false);
    if (VOID.has(t) || (inContainer && !FORMATTING.has(t))) return true;
    if (t === 'a') {
      // An `a` closes one open as its end tag would, and drops it from the stack if that did not; one past a table is
      // past what the scan follows. One a table row above opened may have ended with its cell instead: either way it
      // goes, with the elements above it but the formatting ones, unless a special one stands there
      let k = open.length - 1;
      while (k >= 0 && open[k] !== 'a' && !CELL.has(open[k]!)) k--;
      if (k >= 0 && open[k] === 'a') {
        if (open.some((e, j) => j > k && e === 'table')) return false;
        if (k <= floor) {
          if (open.some((e, j) => j > k && SPECIAL.has(e))) return false;
          retain(k, (e, j) => j > k && FORMATTING.has(e));
        } else if (!endTag('a', sure, inContainer, floor) || open.lastIndexOf('a') >= k) return false;
      }
    } else if (FORMATTING.has(t)) {
      // The parser keeps three of a formatting element's kind to reopen and drops the oldest past that, and a `nobr`
      // closes one in scope: past what the scan follows
      let same = 0;
      for (let k = open.length - 1; k >= 0 && !SCOPE.has(open[k]!); k--) if (open[k] === t) same++;
      if (same >= 3 || (same && t === 'nobr')) return false;
    }
    push(t, !sure);
    return true;
  };
  // An end tag; `sure` that it is one; `floor`: the stack up to there the line may not reach (from a table cell)
  const endTag = (t: string, sure: boolean, inContainer: boolean, floor = -1): boolean => {
    if (UNFOLLOWED.has(t)) return false;
    if (t === 'table' || TABLE_PARTS.has(t)) {
      if (inContainer || floor >= 0 || !sure) return false;
      const c = context();
      // Outside a table the parser drops it; a cell's own end ends it, a row's or section's ends any cell
      if (c < 0) return true;
      if (t === 'table') closeTable(open.lastIndexOf('table'));
      else if (t === open[c] || (open[c] !== 'caption' && open[c] !== 'table' && t === 'tr')) retain(c, () => false);
      // A section's end ends a cell only where that section is open, which the scan does not follow
      else if (open[c] !== 'caption' && open[c] !== 'table' && t !== 'col' && t !== 'colgroup' && TABLE_PARTS.has(t) && !CELL.has(t)) return false;
      return true;
    }
    if (VOID.has(t)) return true;
    if (inContainer && !FORMATTING.has(t)) {
      // In a list or quote: another element's end tag gives up at the item or quote around it, but a special one may end
      // the list or quote itself, or one open around it
      if (!SPECIAL.has(t)) return true;
      return !CONTAINER_TAGS.has(t) && !open.some(e => e === t || (HEADING.test(t) && HEADING.test(e)));
    }
    let k = -1;
    const named = (e: string) => e === t || (HEADING.test(t) && HEADING.test(e));
    if (FORMATTING.has(t) || t === 'p' || SPECIAL.has(t)) {
      k = inScope(named);
      // A list item's end tag gives up at a list
      if (t === 'li' && k >= 0) {
        const list = open.findIndex((e, j) => j > k && (e === 'ul' || e === 'ol'));
        if (list >= 0 && doubt.some((d, j) => d && j > k && (open[j] === 'ul' || open[j] === 'ol'))) return false;
        if (list >= 0) k = -1;
      }
    } else {
      // Another element's: the nearest of its name, unless a special one stands above it
      for (let j = open.length - 1; j >= 0; j--) {
        const found = open[j] === t;
        if (doubt[j] && (found || SPECIAL.has(open[j]!))) return false;
        if (found) { k = j; break; }
        if (SPECIAL.has(open[j]!)) break;
      }
    }
    if (k < 0) return true;
    if (!sure || k <= floor) return false;
    if (doubt[k]) {
      // An element that may be none: the tag may close one of its name below instead, or nothing. A formatting one
      // alone closes no other; another one closes the elements above it, and none of those may be sure
      for (let j = k - 1; j >= 0 && !SCOPE.has(open[j]!); j--) if (named(open[j]!) && !doubt[j]) return false;
      if (!FORMATTING.has(t) && doubt.some((d, j) => j > k && !d && !FORMATTING.has(open[j]!))) return false;
      if (FORMATTING.has(t) && open.some((e, j) => j > k && !SPECIAL.has(e) && !FORMATTING.has(e))) return false;
    }
    if (FORMATTING.has(t)) {
      // The parser's adoption agency moves a formatting element down into each special element above it, eight times
      // at most: past that a copy stays open, to reopen later
      // On its way it drops every element above it but the special and formatting ones (those reopen), and a formatting
      // one more than three elements below a special one too: past what the scan follows
      let specials = 0;
      for (let j = k + 1, run = 0, kept = false; j < open.length; j++) {
        if (!SPECIAL.has(open[j]!)) run++, kept ||= FORMATTING.has(open[j]!);
        else if (run > 3 && kept) return false;
        else specials++, run = 0, kept = false;
      }
      if (specials >= 8) return false;
      retain(k, (e, j) => j > k && (SPECIAL.has(e) || FORMATTING.has(e)));
    } else closeFrom(k);
    return true;
  };

  // A line of an HTML block from `i`, as the HTML tokenizer reads it
  const htmlLine = (line: string, i: number): boolean => {
    if (inTag) {
      i = tagEnd(line, i, inTag);
      if (i < 0) return true;
      // A tag counts once it ends: one left open at the block's end is dropped
      if (inTag.sets) pristine = false;
      inTag = undefined;
    }
    if (inComment) {
      const end = line.indexOf('-->', i);
      if (bang(line, i, end)) return false;
      if (end < 0) return true;
      inComment = false;
      i = end + 3;
    }
    for (let lt = line.indexOf('<', i); lt >= 0; lt = line.indexOf('<', i)) {
      if (!open.length && /[^ \t]/.test(line.slice(i, lt))) rootInline = true;
      if (line.startsWith('<!--', lt)) {
        // `<!-->` and `<!--->` are whole comments
        const abrupt = /^<!---?>/.exec(line.slice(lt));
        const end = abrupt ? lt + abrupt[0].length : line.indexOf('-->', lt + 4);
        if (!abrupt && bang(line, lt + 4, end)) return false;
        if (end < 0) { inComment = true; return true; }
        i = abrupt ? end : end + 3;
        continue;
      }
      const close = line[lt + 1] === '/';
      const at = lt + (close ? 2 : 1);
      if (!/[a-z]/i.test(line[at] ?? '')) {
        // `</>` is dropped, `</` before anything else and `<!` start a bogus comment; `<` before anything else is text
        if (close && line[at] === '>') { i = at + 1; continue; }
        if (close || line[lt + 1] === '!' || line[lt + 1] === '?') return false;
        if (!open.length) rootInline = true;
        i = lt + 1;
        continue;
      }
      // The tokenizer takes a tag's name up to a space, `/` or `>`: one the scan does not know stops it
      const name = /^[^\t />]*/.exec(line.slice(at))![0];
      if (!/^[a-z][a-z0-9-]*$/i.test(name)) return false;
      const t = name.toLowerCase();
      // First in a part the parser drops `</p>` and `</br>`, which the whole makes elements, and a table's part sets its
      // mode to the table's: that part joins the one above. First in the document the whole reads them alike, but a
      // table's part there is past what the scan follows
      if (pristine && (close ? t === 'p' || t === 'br' : TABLE_PARTS.has(t))) {
        if (!parts.length && !close) return false;
        if (parts.length) {
          start -= parts.pop()!.length;
          pristine = false;
          content = true;
        }
      }
      const depth = open.length;
      const root = open[0];
      if (!(close ? endTag(t, true, false) : startTag(t, true, false))) return false;
      // What now stands last at the top level: an element opened or closed there (an end tag with nothing to close is
      // dropped, but `</p>` and `</br>` make an element)
      if (!close) { if (!depth) rootInline = !CLOSES_P.has(t); }
      else if (depth && !open.length) rootInline = !CLOSES_P.has(root!);
      else if (!depth && (t === 'p' || t === 'br')) rootInline = t === 'br';
      const state: InTag = { state: 'name', quote: '', sets: !close && !HEAD.has(t) };
      i = tagEnd(line, at + name.length, state);
      if (i < 0) { inTag = state; return true; }
      if (state.sets) pristine = false;
    }
    if (!inComment && !open.length && /[^ \t]/.test(line.slice(i))) rootInline = true;
    return true;
  };

  // A line of Markdown inline content: a paragraph's or heading's, or one in a list or quote. `code`: the line may be
  // code, as in an indented block or a fence behind quote markers. `raw`: it may be an HTML block's, where neither a
  // backslash nor a backtick hides a tag
  const proseLine = (line: string, inContainer: boolean, code: boolean, raw = false, table = false, block = ''): boolean => {
    // `block`: the element Markdown wraps the line in (`p`, `h1`). Once a tag closed it early, an emphasis, strikethrough
    // or link around a tag after that may leave the tag's element closed or open otherwise than the scan follows
    const gone = () => !!block && !(block === 'p' ? open[paraBase] === 'p' : HEADING.test(open[paraBase] ?? ''));
    // From a link's destination, a reference's label or an image's alt on, and on the paragraph's lines below (a title
    // runs on), what looks like a tag may be none
    const links = murky ? undefined : linkRegions(line, defined);
    const unclear = (i: number) => murky || !!links?.regions.some(([from, to]) => i >= from && i < to);
    // The line may be a table row
    const row = table || PIPE.test(line);
    // A backslash before a character makes it text, in Markdown
    const escaped = (i: number) => {
      if (raw) return false;
      let k = i;
      while (k > 0 && line[k - 1] === '\\') k--;
      return (i - k) % 2 === 1;
    };
    // The code and math spans from `from` to `to`, whose text holds no tags, with `open` the run a span above left open.
    // One that opened on a line above, or is still open at the end, may be none: a block may have started in between, or
    // no end may come. One opened inside what may be a tag or autolink may be none either, and leaves where the rest
    // pair up unclear: on the paragraph's lines below too if `own`
    // A bare URL outside tags with a backtick or dollar in it
    const urlTick = URL_TICK.test(line.replace(/<[a-z/][^<>]*>/gi, ' '));
    const spansIn = (from: number, to: number, open: string, own: boolean) => {
      const found: { from: number; to: number; sure: boolean }[] = [];
      let begin = from;
      let sure = !open;
      let dim = urlTick;
      for (const run of line.slice(from, to).matchAll(/`+|\$+/g)) {
        let at = from + run.index;
        let size = run[0].length;
        if (!open) {
          // An escaped first mark is text; the rest of the run may still open a span
          if (escaped(at)) { at++; size--; }
          if (size < (run[0][0] === '$' ? 2 : 1)) continue;
          open = run[0][0]!.repeat(size);
          begin = at;
          const lt = line.lastIndexOf('<', at - 1);
          sure = !dim && !unclear(at) && (lt < from || line.slice(lt, at).includes('>'));
          if (!sure) {
            dim = true;
            if (own) murky = true;
          }
        } else if (run[0] === open) {
          found.push({ from: begin, to: at, sure });
          open = '';
          // Past a span that may be none, the rest may pair up otherwise
          if (!sure) dim = true;
        }
      }
      if (open) found.push({ from: begin, to, sure: false });
      return { found, open };
    };
    const spans: { from: number; to: number; sure: boolean }[] = [];
    if (raw) tick = '';
    else if (tick || line.includes('`') || line.includes('$$')) {
      const read = spansIn(0, line.length, tick, true);
      tick = read.open;
      spans.push(...read.found);
      if (row) {
        // A table row's cells are split first: a span is one only if it is one in its cell too
        const inCells: typeof spans = [];
        let cell = 0;
        for (let bar = 0; bar <= line.length; bar++) {
          if (bar < line.length && (line[bar] !== '|' || escaped(bar))) continue;
          inCells.push(...spansIn(cell, bar, '', false).found);
          cell = bar + 1;
        }
        for (const s of spans) if (!inCells.some(c => c.sure && c.from === s.from && c.to === s.to)) s.sure = false;
        for (const c of inCells) if (!spans.some(s => s.from === c.from && s.to === c.to)) spans.push({ ...c, sure: false });
      }
    }
    // A link's title may run on to the paragraph's next line
    // A table cell ends what opened in it, and the parser does not reach past it from inside, to what a row above opened
    // or what stood before the table: on what may be a table row, a tag that would reach what stood before the line
    // stops the scan, and what opens on it may have ended with its cell
    const floor = row ? open.length - 1 : -1;
    // An element a Markdown link makes closes an `a` open as its end tag would: text since the last tag that may make one
    // while one is open leaves it in doubt, or past telling with elements above it that this drops
    let read = 0;
    const linkWhileA = (to: number) => {
      if (raw || !open.includes('a') || !LINKISH.test(line.slice(read, to))) return true;
      const k = open.lastIndexOf('a');
      if (open.some((e, j) => j > k && !SPECIAL.has(e) && !FORMATTING.has(e))) return false;
      for (let j = 0; j < open.length; j++) if (open[j] === 'a') doubt[j] = true;
      return true;
    };
    const done = () => {
      if (!linkWhileA(line.length)) return false;
      if (row) for (let j = floor + 1; j < open.length; j++) doubt[j] = true;
      if (links?.carry) murky = true;
      if (block && MARKS.test(line)) marks = true;
      return true;
    };
    if (!line.includes('<')) return done();
    const inSpan = (i: number) => spans.find(s => i >= s.from && i < s.to);
    // A span edge between two points leaves unclear which of the span and what holds it came first
    const crossed = (from: number, to: number) => spans.some(s => (s.from > from && s.from < to) || (s.to > from && s.to < to));
    // A tag running on from the line above holds this line up to a `>`
    const runsTo = tagRuns ? line.indexOf('>') : -1;
    if (tagRuns && runsTo >= 0) tagRuns = false;
    // Whether what looks like a tag at `j` may be none
    const maybeAt = (j: number) => !!inSpan(j) || unclear(j) || code || raw || runsTo >= j || (tagRuns && runsTo < 0);
    // A tag or comment that may be none, over what surely is one if it is none: past telling
    const covers = (from: number, to: number) => {
      for (let j = line.indexOf('<', from + 1); j >= 0 && j < to; j = line.indexOf('<', j + 1)) if (!maybeAt(j)) return true;
      return false;
    };
    let past = 0;
    for (let i = line.indexOf('<'); i >= 0; i = line.indexOf('<', i + 1)) {
      if (i < past) continue;
      const span = inSpan(i);
      if (escaped(i) || span?.sure) continue;
      const maybe = maybeAt(i);
      if (line.startsWith('<!--', i)) {
        // A comment: what it holds is no tag. One that does not end on the line may run on past what the scan follows
        const empty = /^<!---?>/.exec(line.slice(i));
        const end = empty ? i + empty[0].length : line.indexOf('-->', i + 4) + 3;
        if (end < 3 || crossed(i, end) || (!empty && bang(line, i + 4, end - 3)) || (maybe && covers(i, end))) return false;
        past = read = end;
        continue;
      }
      if (MARKUP.test(line.slice(i, i + 3))) {
        // A declaration runs to a `>` either way, which may be on a line below; an instruction or CDATA does not
        const end = line.indexOf('>', i) + 1;
        if (line[i + 1] === '?' || line[i + 2] === '[' || !end || crossed(i, end) || (maybe && covers(i, end))) return false;
        past = read = end;
        continue;
      }
      if (gone() && (marks || MARKS.test(line.slice(0, i)))) return false;
      if (!linkWhileA(i)) return false;
      END_TAG.lastIndex = i;
      const close = END_TAG.exec(line);
      if (close) {
        paraTags = true;
        if (!endTag(close[1]!.toLowerCase(), !maybe, inContainer, floor)) return false;
        read = i + close[0].length;
        continue;
      }
      if (raw && /[a-z/]/i.test(line[i + 1] ?? '')) {
        // What may be an HTML block's: the HTML parser reads more as a tag than Markdown does (`</ul/>`, `<a:b>`, a tag
        // over lines), so anything but a tag both read alike stops the scan
        WHOLE_TAG.lastIndex = i;
        if (!WHOLE_TAG.exec(line)) return false;
      }
      START_NAME.lastIndex = i;
      const name = START_NAME.exec(line);
      if (!name) continue;
      paraTags = true;
      const t = name[1]!.toLowerCase();
      // A table opened in Markdown's inline content takes in the end of an emphasis or link around it, which then
      // cannot close that, so it reopens after the table: past what the scan follows
      if (t === 'table' || TABLE_PARTS.has(t)) return false;
      // In a heading's line an emphasis, strikethrough or link may stand open above the heading, which a heading's start
      // then leaves open
      if (block === 'h1' && HEADING.test(t) && MARKS.test(line.slice(0, i))) return false;
      WHOLE_TAG.lastIndex = i;
      const whole = WHOLE_TAG.exec(line);
      if (whole) {
        past = read = i + whole[0].length;
        if (crossed(i, past) || (maybe && covers(i, past))) return false;
        if (!startTag(t, !maybe, inContainer, floor)) return false;
      } else {
        // Text, or a tag running on to the lines below: what follows on the line may be its attributes
        if (!startTag(t, false, inContainer, floor)) return false;
        murky = true;
        if (!line.includes('>', i)) tagRuns = true;
      }
    }
    return done();
  };

  for (let n = 0, at = 0; n < lines.length; at += lines[n]!.length + 1, n++) {
    scanned = n;
    const line = lineAt(n);
    const col = columnOf(line);
    // A fence or comment opened off the margin may sit in a list item, which a line left of it ends; where the scan
    // then stands is past telling
    const inside = fence ?? comment;
    if (inside && inside.indent > 0 && col >= 0 && col < inside.indent) return rest();
    const mark = FENCE.exec(line);
    if (fence) {
      if (mark && mark[1]!.length >= fence.indent && mark[2]![0] === fence.mark && mark[2]!.length >= fence.size && /^[ \t]*$/.test(mark[3]!)) {
        fence = undefined;
        before = 'end';
      }
      continue;
    }
    if (comment) {
      const end = line.indexOf('-->');
      if (bang(line, 0, end)) return rest();
      if (end >= 0) {
        // Raw HTML after the comment's end is past what the scan follows
        if (/[^ \t]/.test(line.slice(end + 3))) return rest();
        comment = undefined;
        before = 'end';
      }
      continue;
    }
    if (col < 0) {
      // A `pre` block goes on over a blank line, with lines below that look like Markdown as its text; a tag or comment
      // left open in an HTML block is dropped there, past what the scan follows
      if (rawBlock || inTag || inComment) return rest();
      endParagraph();
      before = 'blank';
      html = containerHtml = lazyOk = containerTable = false;
      tick = '';
      // A blank line ends a quote, and a fence in it; a list item may go on past one, but not one it leaves empty
      if (emptyItem) outer = -1, emptyItem = false;
      if (quotedFence?.quotes) quotedFence = undefined;
      continue;
    }
    const lead = line.search(/[^ \t]/);
    const head = line.slice(lead);
    const afterCode = codeOpen;
    codeOpen = false;
    const fresh = before === 'blank' && col === 0 && !TAKEN_IN.test(line);
    if (fresh && !pristine && !open.length && !quotedFence && !unsure && at - start >= part && !FRAGMENT_START.test(line)
      && (!rootInline || !(/^\[|[<|]/.test(line) || delimiterRow(lineAt(n + 1))))) {
      parts.push(markdown.slice(start, at));
      start = at;
      content = false;
      pristine = true;
    }
    // After a blank line, one without markers left of the outermost item's content stands past the list or quote
    if (fresh || (container && before === 'blank' && !nestedIn(col) && !TAKEN_IN.test(head))) {
      container = containerHtml = unsure = false;
      quotedFence = undefined;
    }
    if (html) {
      // An HTML block's line, raw to the HTML parser; a `pre` block ends with the line holding its end tag
      content = true;
      if (!htmlLine(line, 0)) return rest();
      if (rawBlock && RAW_BLOCK_END.test(line)) {
        if (inTag || inComment) return rest();
        rawBlock = html = false;
        before = 'end';
      }
      continue;
    }
    if (container && quotedFence) {
      // A line of a fence in a list item or quote: code, or its end
      const markers = MARKERS.exec(line)![0];
      const quotes = QUOTES.test(markers) ? markers.split('>').length - 1 : -1;
      if (quotedFence.quotes ? quotes === quotedFence.quotes : col >= quotedFence.col) {
        const body = quotedFence.quotes ? line.slice(markers.length) : line;
        const end = /^([ \t]*)(`{3,}|~{3,}|\${2,})[ \t]*$/.exec(body);
        // How far in its marks stand: from the quote markers, or from the fence's own
        const lead = end ? end[1]!.length - (quotedFence.quotes ? 0 : quotedFence.col) : 0;
        inItem = !lazySeen;
        const closes = !!end && end[2]![0] === quotedFence.mark && end[2]!.length >= quotedFence.size;
        if (closes && !end![1]!.includes('\t') && lead <= quotedFence.limit) quotedFence = undefined;
        else if (closes && (end![1]!.includes('\t') || lead <= 3)) {
          // An end past the outermost item's reach may be one in an item nested deeper, and a tab's width depends on
          // where the markers end: past telling
          quotedFence = undefined;
          unsure = true;
          if (!proseLine(line, true, true, true)) return rest();
        } else if (!proseLine(line, true, true, true)) return rest();
        before = 'text';
        continue;
      }
      quotedFence = undefined;
      unsure = true;
    }
    // A comment or a block HTML line left of the outermost item's content, without markers, breaks out of the list or
    // quote (it breaks into a paragraph), and stands as an HTML block on its own
    if (container && col <= 3 && !nestedIn(col) && !MARKERS.exec(line)![0] && (head.startsWith('<!--') || RAW_BLOCK.test(head) || BLOCK_START.test(head))) {
      container = containerHtml = unsure = false;
      quotedFence = undefined;
    }
    if (col <= 3 && head.startsWith('<!--') && !container) {
      // A comment block breaks into a paragraph. One closed as soon as it opens is a comment to the HTML parser and
      // may not be one to Markdown
      endParagraph();
      if (/^<!---?>/.test(head)) return rest();
      const end = head.indexOf('-->', 4);
      if (bang(head, 4, end)) return rest();
      if (end < 0) comment = { indent: col };
      else if (/[^ \t]/.test(head.slice(end + 3))) return rest();
      before = 'end';
      continue;
    }
    if (MAYBE_DEFINITION.test(line) && !NO_DEFINITION.test(line)) {
      // Only a definition of the plain shape, where no paragraph can have been going on, and with no title on the
      // next line, is known to be one. An escape may end or carry on any piece of it
      if (line.includes('\\') || container) return rest();
      const def = DEFINITION.exec(line);
      if (!def || !def[1]!.trim() || before === 'text' || !balanced(def[2]!)) return rest();
      // A title on the next line may belong to it; a table's delimiter row there makes the line the table's header
      const next = lineAt(n + 1);
      if (/^[ \t]*["'(]/.test(next) || delimiterRow(next)) return rest();
      definitions.push({ words: label(def[1]!).split(' '), line });
      endParagraph();
      before = 'definition';
      continue;
    }
    if (LABEL_OPEN.test(line)) {
      // A definition whose label runs on to the lines below, up to the paragraph's end. Whatever came before: a line the
      // scan calls text may have ended its block
      // Up to the paragraph's end, or past the longest label (counted without quote and list markers, which it runs over)
      let text = line;
      let size = 0;
      for (let k = n + 1; k < lines.length && /[^ \t]/.test(lineAt(k)) && size < 1024; k++) {
        text += `\n${lineAt(k)}`;
        size += lineAt(k).replace(/^[ \t]*(?:(?:[-+*>]|\d{1,9}[.)])[ \t]*)*/, '').length;
      }
      if (MAYBE_DEFINITION.test(text) && !NO_DEFINITION.test(text)) return rest();
    }
    content = true;
    // An instruction, declaration or CDATA block runs to its end over blank lines, past what the scan follows
    if (col <= 3 && MARKUP.test(head)) return rest();
    // A table's body ends at a tag alone on its line, at indented code and at any list item, which go on a paragraph,
    // and takes a setext underline as a row: in what may be a table, past telling. Over what may be a delimiter row
    // such a tag breaks into a paragraph as an HTML block
    if (!container && para && tableish && (col >= 4 || (col <= 3 && (LONE_TAG.test(head)
      || (TAKEN_IN.test(head) && !INTERRUPTS.test(line)) || (SETEXT.test(line) && !BREAK.test(line)))))) return rest();
    if (!container && col <= 3 && LONE_TAG.test(head) && (before === 'text' || before === 'definition') && delimiterRow(lineAt(n + 1))) {
      return rest();
    }
    if (!container && col <= 3 && (RAW_BLOCK.test(head) || BLOCK_START.test(head) || (LONE_TAG.test(head) && before !== 'text' && before !== 'definition'))) {
      // An HTML block: one that may break into a paragraph ends it
      endParagraph();
      html = true;
      rawBlock = RAW_BLOCK.test(head);
      tick = '';
      if (!htmlLine(line, 0)) return rest();
      if (rawBlock && RAW_BLOCK_END.test(line)) {
        if (inTag || inComment) return rest();
        rawBlock = html = false;
        before = 'end';
      } else before = 'text';
      continue;
    }
    // Anything else is a Markdown block, whose element sets the parser's mode. In a table outside its cells the parser
    // moves one out in front of the table, and a Markdown table's start ends the table: past what the scan follows
    const c = context();
    if (c >= 0 && open[c] === 'table') return rest();
    pristine = rootInline = false;
    // An HTML block in a list item takes in what looks like a fence on the item's lines, unless a line that may have been
    // a lazy one ended the item: past telling. So is one on the item's lines where the scan lost track of its fences,
    // which may open a fence or end one
    if (mark && nestedIn(col) && (containerHtml ? lazySeen : container && unsure)) return rest();
    // A fence breaks into a paragraph; one off the margin may sit in a list item. A backtick or dollar fence's info
    // string holds none of its own mark
    if (mark && !(containerHtml && nestedIn(col)) && (mark[2]![0] === '~' || !mark[3]!.includes(mark[2]![0]!))) {
      endParagraph();
      if (container && !nestedIn(col)) container = containerHtml = unsure = false;
      if (!container) closeP();
      // No line without markers carries on what stands in an item past its fence
      lazyOk = false;
      fence = { mark: mark[2]![0]!, size: mark[2]!.length, indent: mark[1]!.length };
      tick = '';
      continue;
    }
    // A thematic break left of the outermost item's content, or without quote markers, ends the list or quote, and so do
    // a heading and a fence there, which no paragraph takes in as a lazy line
    if (container && col <= 3 && !nestedIn(col) && (BREAK.test(line) || (!MARKERS.exec(line)![0] && ATX.test(line)))) {
      container = containerHtml = unsure = false;
    }
    if (!container && (BREAK.test(line) || (para && SETEXT.test(line)))) {
      // A thematic break, or a paragraph's setext underline: a heading the scan does not follow if it held a tag
      // One whose start would close a heading the paragraph's did not
      if (para && SETEXT.test(line) && (paraTags || currentHeading(paraBase, false) !== -1)) return rest();
      endParagraph();
      closeP();
      before = 'end';
      continue;
    }
    if (!container && col <= 3 && (para || before === 'text' || before === 'definition' || afterCode ? INTERRUPTS.test(line) : TAKEN_IN.test(head))) {
      // A list or quote: its start closes a paragraph
      endParagraph();
      closeP();
      container = true;
      containerBase = open.length;
      outer = itemOffset(line);
      containerHtml = lazySeen = lazyOk = containerTable = false;
      tick = '';
    }
    if (container) {
      // A fence behind quote or list markers, or one deeper than three columns in an item: the lines up to its end may
      // be code. So may a line indented four columns past its markers
      const markers = MARKERS.exec(line)![0];
      const body = line.slice(markers.length);
      const inner = columnOf(body);
      // Markers start a list item or quote where nothing else can stand: a quote marker or a new list's up to three
      // columns in, an item's in the outermost one up to three columns past its content (an ordered one not numbered 1
      // only after a blank line, as it does not break into a paragraph). Elsewhere they may be text of a paragraph
      // going on, or code
      const first = /^( *)(>|[-+*]|(\d{1,9})[.)])/.exec(line);
      const nested = nestedIn(col);
      // Past a line that may have been a lazy one, and a paragraph at the top level, only markers that may break into a
      // paragraph there start an item for sure
      const breaks = !!first && col <= 3
        && (first[2] === '>' || ((!first[3] || +first[3] === 1) && /^[ \t]+[^ \t]/.test(line.slice(first[0].length))));
      const starts = !!markers && !!first && (first[2] === '>' ? col <= 3
        : nested ? col - outer <= 3 && (!first[3] || +first[3] === 1 || before === 'blank') : col <= 3) && (!lazySeen || breaks);
      if (starts && !nested) outer = itemOffset(line);
      emptyItem = starts && !nested && inner < 0;
      // A new item, or a quote line with nothing on it, ends the paragraph before: spans and link text end with it. A list
      // marker that cannot break into a paragraph (nothing after it, numbered past 1) is text of one going on, so unless
      // a blank line came before, where they end is past telling
      if ((starts && first![2] !== '>') || inner < 0) {
        const breaksIn = !!first && (first[2] === '>' || (inner >= 0 && (!first[3] || +first[3] === 1)));
        if (!breaksIn && before !== 'blank' && (tick || murky || tagRuns)) murky = true;
        else murky = tagRuns = false, tick = '';
      }
      // A line that starts an item or quote is in one, and so is one inside the outermost item. Another may be a lazy
      // line of a paragraph in one, or follow a list or quote it ended: an HTML block there would be outside it
      const within = starts || nested;
      if (!within && !lazyOk) return rest();
      if (starts) lazySeen = false;
      else if (!within) lazySeen = true;
      inItem = within && !lazySeen;
      const text = body.replace(/^[ \t]+/, '');
      if (MARKUP.test(text)) return rest();
      // An HTML block's line, raw to the HTML parser. A comment block ends on the line holding its end
      let rawHere = false;
      if (htmlStart(text)) {
        // Past a line that may have ended the list or quote, one on the left may stand at the top level
        if ((!within || lazySeen) && col <= 3) return rest();
        rawHere = true;
        if (!text.startsWith('<!--') || text.indexOf('-->', 2) < 0) containerHtml = true;
      }
      const fenced = /^([ \t]*)(`{3,}|~{3,}|\${2,})(.*)$/.exec(body);
      const opens = !!fenced && (fenced[2]![0] === '~' || !fenced[3]!.includes(fenced[2]![0]!));
      // In an HTML block a fence may start a new item's code, past what a blank line ends
      if (containerHtml && opens) unsure = true;
      if (opens && !unsure && !containerHtml) {
        const gap = fenced![1]!;
        const fence = { mark: fenced![2]![0]!, size: fenced![2]!.length };
        if (/\t/.test(markers + gap) || (markers && !starts)) unsure = true;
        else if (!markers) {
          // Four columns in or more: a fence in the outermost item, or one nested deeper, or code in the outermost
          if (within && col - outer <= 3) quotedFence = { ...fence, col, limit: outer + 3 - col, quotes: 0 };
          else if (within) unsure = true;
        } else if (gap.length < 5 && !markers.includes('>')) {
          // The fence starts the innermost item's content
          quotedFence = { ...fence, col: markers.length + gap.length, limit: 3, quotes: 0 };
        } else if (gap.length < 5 && outer < 0 && QUOTES.test(markers)) {
          quotedFence = { ...fence, col: -1, limit: 4, quotes: markers.split('>').length - 1 };
        } else if (gap.length < 5) unsure = true;
      }
      if (quotedFence) {
        // Its opening line
      } else if (containerHtml || rawHere || unsure) {
        // An HTML block in an item, up to a blank line, or what the scan cannot tell from one or from code
        if (!proseLine(line, true, true, true)) return rest();
      } else if (!proseLine(line, true, inner >= 4 || (!markers && col >= 4), false, true)) return rest();
      // A paragraph the line leaves open: one it carries on without markers, or starts where no code can stand. A lazy
      // line is no setext underline
      containerTable ||= delimiterRow(text);
      const plain = !quotedFence && !containerHtml && !rawHere && !unsure && !containerTable && inner >= 0
        && !BREAK.test(text) && !(within && SETEXT.test(text)) && !ATX.test(text) && !opens;
      lazyOk = plain && ((lazyOk && !markers) || (markers ? inner < 4 : nestedIn(col) && col - outer < 4));
      before = 'text';
      continue;
    }
    if (ATX.test(line)) {
      // A heading: its `<h1>` closes a paragraph and a heading just opened, and its line ends it
      endParagraph();
      closeP();
      const current = currentHeading(open.length, false);
      if (current === undefined || (current >= 0 && reopens())) return rest();
      if (current >= 0) retain(current, () => false);
      push('h1');
      paraBase = open.length - 1;
      if (!proseLine(line, false, false, false, false, 'h1')) return rest();
      const k = inScope(e => HEADING.test(e));
      // Its end closes the heading nearest the top, which may not be its own if that one may be none
      if (k >= 0 && doubt[k]) return rest();
      // An end tag in it may have closed it before the rest of its line, which then stands inline at the top level
      if (k < paraBase) rootInline = true;
      if (k >= 0) closeFrom(k);
      murky = tagRuns = false;
      tick = '';
      before = 'end';
      continue;
    }
    if (!para && before !== 'definition' && col >= 4) {
      // An indented code block: its `<pre>` closes a paragraph, its lines hold no tags
      closeP();
      codeOpen = true;
      before = 'end';
      continue;
    }
    if (!para) {
      // A paragraph (or a table): its `<p>` closes one open
      closeP();
      paraBase = open.length;
      push('p');
      para = true;
    }
    // A paragraph's line may be a table's row: its header with the delimiter row next, or one past a delimiter row (a
    // body row needs no bar)
    const next = lineAt(n + 1);
    const table = tableish || delimiterRow(next);
    tableish ||= delimiterRow(line);
    // A table's cells end what opens in them, which the paragraph's end does not where a tag closed it early
    if ((table || PIPE.test(line)) && open[paraBase] !== 'p') return rest();
    if (!proseLine(line, false, false, false, table, 'p')) return rest();
    before = 'text';
  }
  if (content || !parts.length) parts.push(markdown.slice(start));
  else parts[parts.length - 1] += markdown.slice(start);
  return parts.length > 1 ? parts.map(withDefinitions) : parts;

  // The definitions the part may name, in document order, so the first of a label still wins: those whose label's
  // words all stand in it. One more renders nothing
  function withDefinitions(text: string): string {
    if (!definitions.length) return text;
    const words = wordsOf(text);
    const lines = definitions.filter(d => d.words.every(w => words.has(w))).map(d => d.line);
    // A byte order mark stays first, where the Markdown parser drops it
    const bom = text.startsWith('\ufeff') ? '\ufeff' : '';
    return lines.length ? `${bom}${lines.join('\n')}\n\n${text.slice(bom.length)}` : text;
  }
}
