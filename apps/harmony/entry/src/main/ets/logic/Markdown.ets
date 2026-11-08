// Markdown → block AST, ported from apps/ios AcpiraMarkdown/AcpiraMarkdown.swift (apps/android logic/Markdown.kt): the
// subset the webview's streamdown pipeline renders for agent prose. GFM blocks (headings, paragraphs, nested lists with
// task items, quotes, fenced code, tables, rules), `$$` display math and the inline set (code, strong, emphasis, strike,
// links, images, autolinks, breaks). Safe against half-streamed syntax: an unclosed fence runs to the end, an unclosed
// delimiter stays literal text (`repairStream` closes the common ones first, as streamdown's remend does). Single-dollar
// math stays off, like the webview ("$5 and $10"). Task items are GFM's (streamdown draws them; the iOS parser reads
// them as plain text).

/** Inline content; `kind`: text, code, strong, em, strike, link, image, br */
export class MdInline {
  kind: string = 'text';
  text: string = '';
  url: string = '';
  children: MdInline[] = [];
}

/** One block; `kind`: heading, p, list, quote, code, table, rule, math */
export class MdBlock {
  kind: string = 'p';
  level: number = 0;
  inlines: MdInline[] = [];
  // list
  ordered: boolean = false;
  start: number = 1;
  items: MdBlock[][] = [];
  /** Per item: -1 no task box, 0 open, 1 done */
  checks: number[] = [];
  // quote
  blocks: MdBlock[] = [];
  // code, math
  lang: string = '';
  text: string = '';
  // table: per column 'none', 'left', 'center', 'right'
  align: string[] = [];
  header: MdInline[][] = [];
  rows: MdInline[][][] = [];
}

function inline(kind: string, text: string): MdInline {
  const i = new MdInline();
  i.kind = kind;
  i.text = text;
  return i;
}

function styled(kind: string, children: MdInline[], url: string = ''): MdInline {
  const i = new MdInline();
  i.kind = kind;
  i.children = children;
  i.url = url;
  return i;
}

function block(kind: string): MdBlock {
  const b = new MdBlock();
  b.kind = kind;
  return b;
}

export function parseMarkdown(source: string): MdBlock[] {
  return new BlockParser(markdownLines(source)).blocks();
}

export function markdownLines(source: string): string[] {
  return source.replace(/\r\n/g, '\n').split('\n');
}

/** Plain text of inline content (copy, alt text) */
export function plainInline(inlines: MdInline[]): string {
  let out = '';
  for (const i of inlines) {
    if (i.kind === 'text' || i.kind === 'code' || i.kind === 'image') {
      out += i.text;
    } else if (i.kind === 'br') {
      out += '\n';
    } else {
      out += plainInline(i.children);
    }
  }
  return out;
}

// The parser's patterns, compiled once: they run on every line of a streamed reply on every patch
const headingRe = /^ {0,3}(#{1,6})(?:[ \t]+(.*?))?[ \t]*#*[ \t]*$/;
const ruleRe = /^ {0,3}(?:(?:\*[ \t]*){3,}|(?:-[ \t]*){3,}|(?:_[ \t]*){3,})$/;
const listMarkerRe = /^( *)([-*+]|\d{1,9}[.)])( +|$)/;
const tableDelimiterRe = /^ {0,3}\|?[ \t]*:?-+:?[ \t]*(?:\|[ \t]*:?-+:?[ \t]*)*\|?[ \t]*$/;
const autolinkRe = /^[a-zA-Z][a-zA-Z0-9+.-]{1,31}:[^\s<>]*$/;
const bareUrlRe = /^(?:https?:\/\/|www\.)[^\s<]+/;
const taskRe = /^\[([ xX])\](?:[ \t]+|$)/;

class ListMarker {
  indent: number = 0;
  ordered: boolean = false;
  num: number = 1;
  content: number = 0;
}

class Fence {
  marker: string = '';
  lang: string = '';
}

function indentWidth(line: string): number {
  let n = 0;
  for (let i = 0; i < line.length; i++) {
    const c = line.charAt(i);
    if (c === ' ') {
      n++;
    } else if (c === '\t') {
      n += 4;
    } else {
      break;
    }
  }
  return n;
}

function isBlankLine(line: string): boolean {
  return /^[ \t]*$/.test(line);
}

function trimSpaces(s: string): string {
  return s.replace(/^[ \t]+/, '').replace(/[ \t]+$/, '');
}

function fenceOf(line: string): Fence | undefined {
  if (indentWidth(line) >= 4) {
    return undefined;
  }
  const trimmed = line.replace(/^ +/, '');
  for (const mark of ['```', '~~~']) {
    if (!trimmed.startsWith(mark)) {
      continue;
    }
    let run = 0;
    while (run < trimmed.length && trimmed.charAt(run) === mark.charAt(0)) {
      run++;
    }
    const info = trimmed.slice(run).trim();
    if (mark === '```' && info.includes('`')) {
      return undefined;
    }
    const f = new Fence();
    f.marker = trimmed.slice(0, run);
    f.lang = info.length === 0 ? '' : info.split(' ')[0];
    return f;
  }
  return undefined;
}

function listMarkerOf(line: string): ListMarker | undefined {
  const m = line.match(listMarkerRe);
  if (m === null) {
    return undefined;
  }
  const marker = new ListMarker();
  marker.indent = m[1].length;
  marker.ordered = /\d/.test(m[2].charAt(0));
  marker.num = marker.ordered ? (parseInt(m[2].slice(0, -1)) || 1) : 1;
  const spaces = Math.min(Math.max(m[3].length, 1), 4);
  marker.content = marker.indent + m[2].length + spaces;
  return marker;
}

function isTableDelimiter(line: string): boolean {
  return tableDelimiterRe.test(line) && line.includes('-');
}

function cells(line: string): string[] {
  let s = trimSpaces(line);
  if (s.startsWith('|')) {
    s = s.slice(1);
  }
  if (s.endsWith('|') && !s.endsWith('\\|')) {
    s = s.slice(0, -1);
  }
  // Split on unescaped pipes outside inline code
  const out: string[] = [];
  let cell = '';
  let inCode = false;
  let prev = ' ';
  for (let i = 0; i < s.length; i++) {
    const c = s.charAt(i);
    if (c === '`') {
      inCode = !inCode;
    }
    if (c === '|' && !inCode && prev !== '\\') {
      out.push(cell);
      cell = '';
    } else {
      cell += c;
    }
    prev = c;
  }
  out.push(cell);
  return out.map((c: string) => trimSpaces(c).split('\\|').join('|'));
}

class BlockParser {
  i: number = 0;
  private lines: string[];

  constructor(lines: string[]) {
    this.lines = lines;
  }

  /** Whether a line can interrupt a paragraph */
  private startsBlock(line: string): boolean {
    if (fenceOf(line) !== undefined || headingRe.test(line) || ruleRe.test(line)) {
      return true;
    }
    if (line.replace(/^ +/, '').startsWith('>') || trimSpaces(line).startsWith('$$')) {
      return true;
    }
    const m = listMarkerOf(line);
    if (m === undefined) {
      return false;
    }
    // An ordered list interrupts a paragraph only from 1; an empty item never does
    return (!m.ordered || m.num === 1) && !isBlankLine(line.slice(Math.min(m.content, line.length)));
  }

  blocks(): MdBlock[] {
    const out: MdBlock[] = [];
    const lines = this.lines;
    while (this.i < lines.length) {
      const line = lines[this.i];
      if (isBlankLine(line)) {
        this.i++;
        continue;
      }
      const f = fenceOf(line);
      if (f !== undefined) {
        out.push(this.fencedCode(f));
        continue;
      }
      if (trimSpaces(line).startsWith('$$')) {
        out.push(this.math());
        continue;
      }
      const h = line.match(headingRe);
      if (h !== null) {
        const b = block('heading');
        b.level = h[1].length;
        b.inlines = parseInline(h[2] ?? '');
        out.push(b);
        this.i++;
        continue;
      }
      if (ruleRe.test(line)) {
        out.push(block('rule'));
        this.i++;
        continue;
      }
      if (line.replace(/^ +/, '').startsWith('>')) {
        out.push(this.quote());
        continue;
      }
      if (listMarkerOf(line) !== undefined) {
        out.push(this.list());
        continue;
      }
      if (this.i + 1 < lines.length && line.includes('|') && isTableDelimiter(lines[this.i + 1])) {
        out.push(this.table());
        continue;
      }
      const p = this.paragraph();
      // A paragraph of nothing but `<br>` tags adds no block (a model stuck repeating the tag)
      if (p.inlines.every(isBlankInline)) {
        continue;
      }
      out.push(p);
    }
    return out;
  }

  private fencedCode(f: Fence): MdBlock {
    const lines = this.lines;
    const indent = indentWidth(lines[this.i]);
    this.i++;
    const body: string[] = [];
    while (this.i < lines.length) {
      const line = lines[this.i];
      const trimmed = trimSpaces(line);
      if (trimmed.startsWith(f.marker) && trimmed.split('').every((c: string) => c === f.marker.charAt(0))) {
        this.i++;
        break;
      }
      // Strip up to the fence's own indent
      let drop = 0;
      while (drop < indent && drop < line.length && line.charAt(drop) === ' ') {
        drop++;
      }
      body.push(line.slice(drop));
      this.i++;
    }
    const b = block('code');
    b.lang = f.lang;
    b.text = body.join('\n');
    return b;
  }

  private math(): MdBlock {
    const lines = this.lines;
    let first = trimSpaces(lines[this.i]).slice(2);
    const b = block('math');
    // `$$ x $$` on one line
    if (first.endsWith('$$') && first.length >= 2) {
      first = first.slice(0, -2);
      this.i++;
      b.text = trimSpaces(first);
      return b;
    }
    const body: string[] = first.length === 0 ? [] : [first];
    this.i++;
    while (this.i < lines.length) {
      const tl = trimSpaces(lines[this.i]);
      if (tl.endsWith('$$')) {
        const rest = tl.slice(0, -2);
        if (rest.length > 0) {
          body.push(rest);
        }
        this.i++;
        break;
      }
      body.push(lines[this.i]);
      this.i++;
    }
    b.text = body.join('\n').trim();
    return b;
  }

  private quote(): MdBlock {
    const lines = this.lines;
    const inner: string[] = [];
    while (this.i < lines.length) {
      const line = lines[this.i];
      const stripped = line.replace(/^ +/, '');
      if (stripped.startsWith('>')) {
        let rest = stripped.slice(1);
        if (rest.startsWith(' ')) {
          rest = rest.slice(1);
        }
        inner.push(rest);
        this.i++;
      } else if (!isBlankLine(line) && inner.length > 0 && !isBlankLine(inner[inner.length - 1]) && !this.startsBlock(line)) {
        // Lazy continuation of the quoted paragraph
        inner.push(line);
        this.i++;
      } else {
        break;
      }
    }
    const b = block('quote');
    b.blocks = new BlockParser(inner).blocks();
    return b;
  }

  private list(): MdBlock {
    const lines = this.lines;
    const first = listMarkerOf(lines[this.i]) as ListMarker;
    const b = block('list');
    b.ordered = first.ordered;
    b.start = first.num;
    while (this.i < lines.length) {
      const m = listMarkerOf(lines[this.i]);
      if (m === undefined || m.ordered !== first.ordered || m.indent >= first.content || m.indent > first.indent + 3) {
        break;
      }
      // The item's first line, dedented to its content column; a GFM task box leads it
      let head = lines[this.i].slice(Math.min(m.content, lines[this.i].length));
      const task = head.match(taskRe);
      let check = -1;
      if (task !== null) {
        check = task[1] === ' ' ? 0 : 1;
        head = head.slice(task[0].length);
      }
      const body: string[] = [head];
      this.i++;
      let blankRun = false;
      while (this.i < lines.length) {
        const line = lines[this.i];
        if (isBlankLine(line)) {
          body.push('');
          blankRun = true;
          this.i++;
          continue;
        }
        if (indentWidth(line) >= m.content) {
          body.push(line.slice(Math.min(m.content, line.length)));
          blankRun = false;
          this.i++;
          continue;
        }
        // A sibling or an outer block ends the item; a lazy line continues its paragraph
        if (blankRun || listMarkerOf(line) !== undefined || this.startsBlock(line)) {
          break;
        }
        body.push(trimSpaces(line));
        this.i++;
      }
      while (body.length > 0 && isBlankLine(body[body.length - 1])) {
        body.pop();
      }
      b.items.push(new BlockParser(body).blocks());
      b.checks.push(check);
      // A blank line between items keeps the list going; anything else stops it
      let j = this.i;
      while (j < lines.length && isBlankLine(lines[j])) {
        j++;
      }
      const next = j < lines.length ? listMarkerOf(lines[j]) : undefined;
      if (next !== undefined && next.ordered === first.ordered && next.indent < first.content) {
        this.i = j;
      } else {
        break;
      }
    }
    return b;
  }

  private table(): MdBlock {
    const lines = this.lines;
    const b = block('table');
    b.header = cells(lines[this.i]).map((c: string) => parseInline(c));
    b.align = cells(lines[this.i + 1]).map((spec: string) => {
      if (spec.startsWith(':') && spec.endsWith(':')) {
        return 'center';
      }
      if (spec.startsWith(':')) {
        return 'left';
      }
      return spec.endsWith(':') ? 'right' : 'none';
    });
    this.i += 2;
    while (this.i < lines.length && !isBlankLine(lines[this.i]) && !this.startsBlock(lines[this.i])) {
      const row = cells(lines[this.i]).map((c: string) => parseInline(c));
      // Rows take the header's width
      while (row.length < b.header.length) {
        row.push([]);
      }
      b.rows.push(row.slice(0, b.header.length));
      this.i++;
    }
    return b;
  }

  private paragraph(): MdBlock {
    const lines = this.lines;
    const text: string[] = [lines[this.i]];
    this.i++;
    while (this.i < lines.length && !isBlankLine(lines[this.i]) && !this.startsBlock(lines[this.i])) {
      if (this.i + 1 < lines.length && lines[this.i].includes('|') && isTableDelimiter(lines[this.i + 1])) {
        break;
      }
      text.push(lines[this.i]);
      this.i++;
    }
    const b = block('p');
    b.inlines = parseInline(text.map((l: string) => l.replace(/^ +/, '')).join('\n'));
    return b;
  }
}

function isBlankInline(i: MdInline): boolean {
  return i.kind === 'br' || (i.kind === 'text' && i.text.trim().length === 0);
}

export function parseInline(text: string): MdInline[] {
  return mergeText(new InlineParser(text).parse(''));
}

/** Adjacent text runs joined into one */
function mergeText(items: MdInline[]): MdInline[] {
  const out: MdInline[] = [];
  for (const item of items) {
    const prev = out.length > 0 ? out[out.length - 1] : undefined;
    if (item.kind === 'text' && prev !== undefined && prev.kind === 'text') {
      out[out.length - 1] = inline('text', prev.text + item.text);
    } else if (item.kind === 'text' && item.text.length === 0) {
      continue;
    } else {
      out.push(item);
    }
  }
  return out;
}

const punctuation = '!"#$%&\'()*+,-./:;<=>?@[\\]^_`{|}~';
const urlTrail = '.,:;!?)"\'*_~';
const cjkStop = '，。；：！？、）》」』';

function letterOrNumber(c: string): boolean {
  return /[\p{L}\p{N}]/u.test(c);
}

function isSpace(c: string): boolean {
  return /\s/.test(c);
}

class InlineParser {
  i: number = 0;
  private chars: string;

  constructor(chars: string) {
    this.chars = chars;
  }

  /** Parses until the closing delimiter `close` (consumed by the caller) or the end */
  parse(close: string): MdInline[] {
    const chars = this.chars;
    const out: MdInline[] = [];
    let text = '';
    const flush = () => {
      if (text.length > 0) {
        out.push(inline('text', text));
        text = '';
      }
    };
    while (this.i < chars.length) {
      if (close.length > 0 && chars.startsWith(close, this.i)) {
        flush();
        return out;
      }
      const c = chars.charAt(this.i);
      // Escapes
      if (c === '\\' && this.i + 1 < chars.length) {
        const n = chars.charAt(this.i + 1);
        if (n === '\n') {
          flush();
          out.push(inline('br', ''));
          this.i += 2;
          continue;
        }
        if (punctuation.includes(n)) {
          text += n;
          this.i += 2;
          continue;
        }
      }
      // Hard break: two trailing spaces; a soft break renders as a space
      if (c === '\n') {
        if (text.endsWith('  ')) {
          text = text.slice(0, -2);
          flush();
          out.push(inline('br', ''));
        } else if (text.length === 0 && out.length > 0 && out[out.length - 1].kind === 'br') {
          // A newline right after a break starts the next line without a leading space
        } else {
          text = text.replace(/ +$/, '') + ' ';
        }
        this.i++;
        while (this.i < chars.length && chars.charAt(this.i) === ' ') {
          this.i++;
        }
        continue;
      }
      if (c === '`') {
        const code = this.codeSpan();
        if (code !== undefined) {
          flush();
          out.push(inline('code', code));
          continue;
        }
      }
      if (c === '!' && chars.charAt(this.i + 1) === '[') {
        const image = this.imageAt();
        if (image !== undefined) {
          flush();
          out.push(image);
          continue;
        }
      }
      if (c === '[') {
        const link = this.linkAt();
        if (link !== undefined) {
          flush();
          out.push(link);
          continue;
        }
      }
      if (c === '<') {
        const width = this.htmlBreak();
        if (width !== undefined) {
          flush();
          out.push(inline('br', ''));
          this.i += width;
          continue;
        }
        const auto = this.autolink();
        if (auto !== undefined) {
          flush();
          out.push(auto);
          continue;
        }
      }
      if (c === 'h' || c === 'w') {
        const bare = this.bareUrl();
        if (bare !== undefined) {
          flush();
          out.push(bare);
          continue;
        }
      }
      if (c === '*' || c === '_' || c === '~') {
        const s = this.delimited();
        if (s !== undefined) {
          flush();
          out.push(s);
          continue;
        }
        // A run that opens nothing stays literal
        const run = this.runLength(this.i, c);
        text += c.repeat(run);
        this.i += run;
        continue;
      }
      text += c;
      this.i++;
    }
    flush();
    return out;
  }

  private runLength(index: number, c: string): number {
    let n = 0;
    while (index + n < this.chars.length && this.chars.charAt(index + n) === c) {
      n++;
    }
    return n;
  }

  private codeSpan(): string | undefined {
    const chars = this.chars;
    const run = this.runLength(this.i, '`');
    let j = this.i + run;
    while (j < chars.length) {
      if (chars.charAt(j) === '`') {
        const close = this.runLength(j, '`');
        if (close === run) {
          let body = chars.slice(this.i + run, j).replace(/\n/g, ' ');
          if (body.length >= 2 && body.startsWith(' ') && body.endsWith(' ') && body.trim().length > 0) {
            body = body.slice(1, -1);
          }
          this.i = j + close;
          return body;
        }
        j += close;
      } else {
        j++;
      }
    }
    return undefined;
  }

  /** `**strong**`, `*em*`, `_em_`, `__strong__`, `~~strike~~`, `***both***` */
  private delimited(): MdInline | undefined {
    const chars = this.chars;
    const c = chars.charAt(this.i);
    const run = this.runLength(this.i, c);
    // The opener must be followed by non-space
    if (this.i + run >= chars.length || isSpace(chars.charAt(this.i + run))) {
      return undefined;
    }
    // Intraword underscores stay literal
    if (c === '_' && this.i > 0 && letterOrNumber(chars.charAt(this.i - 1))) {
      return undefined;
    }
    if (c === '~' && run !== 2) {
      return undefined;
    }
    const width = c === '~' ? 2 : Math.min(run, 3);
    const delimiter = c.repeat(width);
    let j = this.i + width;
    while (j < chars.length) {
      const cj = chars.charAt(j);
      if (cj === '`') {
        // Skip code spans
        const r = this.runLength(j, '`');
        let k = j + r;
        let closed = false;
        while (k < chars.length) {
          if (chars.charAt(k) === '`' && this.runLength(k, '`') === r) {
            closed = true;
            break;
          }
          k++;
        }
        j = closed ? k + r : j + r;
        continue;
      }
      if (cj === '\\') {
        j += 2;
        continue;
      }
      if (cj === c && chars.startsWith(delimiter, j) && !isSpace(chars.charAt(j - 1))) {
        const after = j + width;
        const next = this.runLength(j, c);
        // For a single delimiter, a longer run is a different (strong) closer
        if (width === 1 && next === 2) {
          j += 2;
          continue;
        }
        if (c === '_' && after < chars.length && letterOrNumber(chars.charAt(after))) {
          j++;
          continue;
        }
        const inner = chars.slice(this.i + width, j);
        this.i = after;
        const content = parseInline(inner);
        if (c === '~') {
          return styled('strike', content);
        }
        if (width === 3) {
          return styled('strong', [styled('em', content)]);
        }
        return width === 2 ? styled('strong', content) : styled('em', content);
      }
      j++;
    }
    return undefined;
  }

  /** `[label](url "title")` */
  private linkAt(): MdInline | undefined {
    const chars = this.chars;
    let depth = 0;
    let j = this.i;
    while (j < chars.length) {
      const cj = chars.charAt(j);
      if (cj === '\\') {
        j += 2;
        continue;
      }
      if (cj === '[') {
        depth++;
      }
      if (cj === ']') {
        depth--;
        if (depth === 0) {
          break;
        }
      }
      j++;
    }
    if (j + 1 >= chars.length || chars.charAt(j) !== ']' || chars.charAt(j + 1) !== '(') {
      return undefined;
    }
    let k = j + 2;
    let parens = 1;
    while (k < chars.length) {
      if (chars.charAt(k) === '(') {
        parens++;
      }
      if (chars.charAt(k) === ')') {
        parens--;
        if (parens === 0) {
          break;
        }
      }
      k++;
    }
    if (k >= chars.length) {
      return undefined;
    }
    const label = chars.slice(this.i + 1, j);
    let target = trimSpaces(chars.slice(j + 2, k));
    // `<a path with spaces>` keeps its spaces; otherwise a space starts the title
    const closeAngle = target.indexOf('>');
    if (target.startsWith('<') && closeAngle >= 0) {
      target = target.slice(1, closeAngle);
    } else if (target.indexOf(' ') >= 0) {
      target = target.slice(0, target.indexOf(' '));
    }
    this.i = k + 1;
    return styled('link', parseInline(label), target);
  }

  /** `![alt](src "title")`: a link's syntax after the bang; the bang stays text when no link follows */
  private imageAt(): MdInline | undefined {
    const start = this.i;
    this.i++;
    const link = this.linkAt();
    if (link === undefined) {
      this.i = start;
      return undefined;
    }
    const image = inline('image', plainInline(link.children));
    image.url = link.url;
    return image;
  }

  /** The length of a `<br>`, `<br/>` or `<br />` tag at `i`, any case; other tags stay literal text */
  private htmlBreak(): number | undefined {
    const chars = this.chars;
    if (this.i + 3 >= chars.length || chars.charAt(this.i + 1).toLowerCase() !== 'b' || chars.charAt(this.i + 2).toLowerCase() !== 'r') {
      return undefined;
    }
    let j = this.i + 3;
    while (j < chars.length && chars.charAt(j) === ' ') {
      j++;
    }
    if (j < chars.length && chars.charAt(j) === '/') {
      j++;
    }
    if (j >= chars.length || chars.charAt(j) !== '>') {
      return undefined;
    }
    return j + 1 - this.i;
  }

  private autolink(): MdInline | undefined {
    const close = this.chars.indexOf('>', this.i);
    if (close < 0) {
      return undefined;
    }
    const body = this.chars.slice(this.i + 1, close);
    if (!autolinkRe.test(body)) {
      return undefined;
    }
    this.i = close + 1;
    return styled('link', [inline('text', body)], body);
  }

  /** GFM bare `https://…` / `www.…` links, trailing punctuation excluded */
  private bareUrl(): MdInline | undefined {
    const chars = this.chars;
    if (this.i > 0 && letterOrNumber(chars.charAt(this.i - 1))) {
      return undefined;
    }
    const m = chars.slice(this.i).match(bareUrlRe);
    if (m === null) {
      return undefined;
    }
    let url = m[0];
    while (url.length > 0 && urlTrail.includes(url.charAt(url.length - 1))) {
      // Keep a closing paren that balances one inside the URL
      if (url.endsWith(')') && url.split('(').length >= url.split(')').length) {
        break;
      }
      url = url.slice(0, -1);
    }
    // CJK punctuation ends the URL as well
    for (let k = 0; k < url.length; k++) {
      if (cjkStop.includes(url.charAt(k))) {
        url = url.slice(0, k);
        break;
      }
    }
    if (url.length === 0) {
      return undefined;
    }
    this.i += url.length;
    return styled('link', [inline('text', url)], url.startsWith('www.') ? `http://${url}` : url);
  }
}

/**
 * The streamed tail repaired before parsing (streamdown's remend, default mode): a fence left open is closed, and in the
 * last paragraph an unmatched `**`, `~~`, `` ` `` or `*` closes at the end, and a link still missing its target shows
 * its label. The finished text is never repaired, so a literal stays literal once the stream ends
 */
export function repairStream(text: string): string {
  const fences = (text.match(/^ {0,3}(```|~~~)/gm) ?? []).length;
  if (fences % 2 === 1) {
    return text;
  }
  const cut = text.lastIndexOf('\n\n');
  const head = cut >= 0 ? text.slice(0, cut + 2) : '';
  let tail = cut >= 0 ? text.slice(cut + 2) : text;
  // An unfinished link: `[label](partial` or `[label]` with nothing after it yet
  tail = tail.replace(/\[([^\]\n]*)\]\([^)\n]*$/, '$1').replace(/\[([^\]\n]*)$/, '$1');
  // Code spans first: markers inside an open span are code, not emphasis
  const ticks = (tail.match(/`/g) ?? []).length;
  if (ticks % 2 === 1) {
    return head + tail + '`';
  }
  const bare = tail.replace(/`[^`]*`/g, '');
  let close = '';
  if ((bare.match(/\*\*/g) ?? []).length % 2 === 1) {
    close += '**';
  }
  if ((bare.match(/~~/g) ?? []).length % 2 === 1) {
    close += '~~';
  }
  const singles = bare.replace(/\*\*/g, '').replace(/^\s*[*-]\s/gm, '');
  if ((singles.match(/\*/g) ?? []).length % 2 === 1 && !/\*\s*$/.test(tail)) {
    close = '*' + close;
  }
  if (close.length > 0 && /\S$/.test(tail)) {
    return head + tail + close;
  }
  return head + tail;
}
