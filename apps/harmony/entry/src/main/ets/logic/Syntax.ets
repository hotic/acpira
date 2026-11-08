// Code colours. The webview, iOS (JavaScriptCore) and Android (the AndroidX JavaScript sandbox) all run shiki with the
// webview's TextMate grammars and the light-plus / dark-plus themes (apps/ios Logic/Syntax.swift). OpenHarmony has no
// TextMate engine and its ArkTS runtime cannot load shiki's oniguruma WASM, so this is a small built-in tokenizer for
// the common languages, colouring each token by the scope dark-plus / light-plus would give it (`role`, resolved to
// `syn_*` colours in resources so it follows the colour mode). Unknown languages stay plain.

/** A run of one line in one role; role '' is the card's own ink */
export class CodeToken {
  text: string = '';
  role: string = '';
}

/** The grammar a fence's info string or a file's extension asks for (codeLanguage.ts), '' when none is known */
export function codeLanguage(name: string | undefined): string {
  const raw = (name ?? '').toLowerCase().trim();
  const ext = raw.includes('.') ? raw.slice(raw.lastIndexOf('.') + 1) : raw;
  switch (ext) {
    case 'ts':
    case 'typescript':
    case 'tsx':
    case 'js':
    case 'javascript':
    case 'jsx':
    case 'mjs':
    case 'cjs':
    case 'mts':
    case 'cts':
    case 'ets':
    case 'arkts':
      return 'js';
    case 'py':
    case 'python':
      return 'py';
    case 'rs':
    case 'rust':
      return 'rust';
    case 'go':
    case 'golang':
      return 'go';
    case 'swift':
      return 'swift';
    case 'kt':
    case 'kts':
    case 'kotlin':
      return 'kotlin';
    case 'java':
      return 'java';
    case 'c':
    case 'h':
    case 'cc':
    case 'cpp':
    case 'c++':
    case 'hpp':
    case 'cs':
    case 'csharp':
    case 'm':
    case 'mm':
    case 'objc':
      return 'c';
    case 'json':
    case 'jsonc':
    case 'json5':
      return 'json';
    case 'sh':
    case 'bash':
    case 'zsh':
    case 'shell':
    case 'console':
    case 'shellscript':
    case 'fish':
    case 'ps1':
    case 'powershell':
      return 'sh';
    case 'yaml':
    case 'yml':
      return 'yaml';
    case 'toml':
    case 'ini':
    case 'conf':
      return 'toml';
    case 'css':
    case 'scss':
    case 'less':
      return 'css';
    case 'html':
    case 'htm':
    case 'xml':
    case 'svg':
    case 'vue':
    case 'plist':
      return 'html';
    case 'sql':
      return 'sql';
    case 'diff':
    case 'patch':
      return 'diff';
    default:
      return '';
  }
}

function words(list: string): Set<string> {
  return new Set<string>(list.split(' '));
}

const control = words('if else for while do switch case default break continue return throw try catch finally await yield ' +
  'import export from as match loop in of when guard defer goto elif except raise with pass del lambda async then fi esac done');
const keywordsJs = words('const let var function class extends implements interface type enum new delete typeof instanceof ' +
  'void this super static public private protected readonly abstract declare namespace module get set keyof infer is struct');
const keywordsRust = words('fn let mut pub struct enum impl trait use mod crate self Self super where move ref static const ' +
  'unsafe extern type dyn as async');
const keywordsGo = words('func var const type struct interface map chan package go select range');
const keywordsSwift = words('func var let class struct enum protocol extension init deinit self Self static private public ' +
  'internal fileprivate open override mutating some any inout typealias associatedtype weak lazy final subscript');
const keywordsKotlin = words('fun val var class object interface data sealed enum companion override private public internal ' +
  'protected open abstract suspend inline lateinit by init this super typealias');
const keywordsJava = words('class interface enum extends implements new public private protected static final abstract void ' +
  'this super synchronized volatile transient native package');
const keywordsC = words('int char float double long short unsigned signed void struct union enum typedef static extern const ' +
  'auto register volatile sizeof inline class namespace template typename public private protected virtual using new delete ' +
  'bool string var');
const keywordsPy = words('def class global nonlocal and or not is None True False self');
const keywordsSh = words('function local export readonly declare unset alias source echo cd');
const keywordsSql = words('select from where insert into update delete create table drop alter index join left right inner outer ' +
  'on group by order having limit offset values set as and or not null is in like primary key foreign references default ' +
  'distinct union all case when then else end begin commit returning');
const constants = words('true false null undefined nil None True False NaN Infinity');

class Lang {
  lineComment: string = '';
  blockComment: boolean = false;
  hashComment: boolean = false;
  templates: boolean = false;
  tripleQuotes: boolean = false;
  keywords: Set<string> = new Set<string>();
  caseless: boolean = false;
}

function lang(id: string): Lang | undefined {
  const l = new Lang();
  switch (id) {
    case 'js':
      l.lineComment = '//';
      l.blockComment = true;
      l.templates = true;
      l.keywords = keywordsJs;
      return l;
    case 'rust':
      l.lineComment = '//';
      l.blockComment = true;
      l.keywords = keywordsRust;
      return l;
    case 'go':
      l.lineComment = '//';
      l.blockComment = true;
      l.templates = true;
      l.keywords = keywordsGo;
      return l;
    case 'swift':
      l.lineComment = '//';
      l.blockComment = true;
      l.tripleQuotes = true;
      l.keywords = keywordsSwift;
      return l;
    case 'kotlin':
      l.lineComment = '//';
      l.blockComment = true;
      l.tripleQuotes = true;
      l.keywords = keywordsKotlin;
      return l;
    case 'java':
      l.lineComment = '//';
      l.blockComment = true;
      l.keywords = keywordsJava;
      return l;
    case 'c':
      l.lineComment = '//';
      l.blockComment = true;
      l.keywords = keywordsC;
      return l;
    case 'py':
      l.hashComment = true;
      l.tripleQuotes = true;
      l.keywords = keywordsPy;
      return l;
    case 'sh':
      l.hashComment = true;
      l.keywords = keywordsSh;
      return l;
    case 'sql':
      l.lineComment = '--';
      l.blockComment = true;
      l.keywords = keywordsSql;
      l.caseless = true;
      return l;
    default:
      return undefined;
  }
}

const numberRe = /^(?:0[xX][\da-fA-F_]+|0[bB][01_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?)[a-zA-Z_]*/;
const identRe = /^[A-Za-z_$][\w$]*/;

class Tokens {
  out: CodeToken[] = [];

  push(text: string, role: string): void {
    if (text.length === 0) {
      return;
    }
    const last = this.out.length > 0 ? this.out[this.out.length - 1] : undefined;
    if (last !== undefined && last.role === role) {
      last.text += text;
      return;
    }
    const token = new CodeToken();
    token.text = text;
    token.role = role;
    this.out.push(token);
  }
}

/** The end of a quoted string starting at `i` (its delimiter included), or the line's end when it never closes */
function stringEnd(code: string, i: number, quote: string, multiline: boolean): number {
  let j = i + quote.length;
  while (j < code.length) {
    if (code.startsWith(quote, j)) {
      return j + quote.length;
    }
    const c = code.charAt(j);
    if (c === '\\') {
      j += 2;
      continue;
    }
    if (c === '\n' && !multiline) {
      return j;
    }
    j++;
  }
  return code.length;
}

function lineEnd(code: string, i: number): number {
  const end = code.indexOf('\n', i);
  return end < 0 ? code.length : end;
}

/** The C-family / scripting tokenizer: comments, strings, numbers, then identifiers by what they are */
function tokenizeCode(code: string, l: Lang, langId: string): CodeToken[] {
  const tokens = new Tokens();
  let i = 0;
  let lineStart = true;
  while (i < code.length) {
    const c = code.charAt(i);
    const rest = code.slice(i, i + 3);
    if (c === '\n') {
      tokens.push(c, '');
      lineStart = true;
      i++;
      continue;
    }
    if (c === ' ' || c === '\t') {
      tokens.push(c, '');
      i++;
      continue;
    }
    if ((l.lineComment.length > 0 && code.startsWith(l.lineComment, i)) || (l.hashComment && c === '#' &&
      !(langId === 'sh' && i > 0 && /\S/.test(code.charAt(i - 1))))) {
      const end = lineEnd(code, i);
      tokens.push(code.slice(i, end), 'comment');
      i = end;
      continue;
    }
    if (l.blockComment && code.startsWith('/*', i)) {
      const close = code.indexOf('*/', i + 2);
      const end = close < 0 ? code.length : close + 2;
      tokens.push(code.slice(i, end), 'comment');
      i = end;
      continue;
    }
    if (l.tripleQuotes && (rest === '"""' || rest === "'''")) {
      const end = stringEnd(code, i, rest, true);
      tokens.push(code.slice(i, end), 'string');
      i = end;
      continue;
    }
    if (c === '"' || c === "'" || (c === '`' && l.templates)) {
      // A Rust lifetime ('a) is no string
      if (c === "'" && langId === 'rust' && /^'[a-z_]\w*(?!')/.test(code.slice(i, i + 32)) && code.charAt(i + 2) !== "'") {
        const m = code.slice(i).match(/^'[a-z_]\w*/);
        const text = m !== null ? m[0] : c;
        tokens.push(text, 'keyword');
        i += text.length;
        continue;
      }
      const end = stringEnd(code, i, c, c === '`');
      tokens.push(code.slice(i, end), 'string');
      i = end;
      lineStart = false;
      continue;
    }
    if (langId === 'sh' && c === '$') {
      const m = code.slice(i).match(/^\$(?:\{[^}\n]*\}|[A-Za-z_]\w*|[@*#?$!\d])/);
      if (m !== null) {
        tokens.push(m[0], 'variable');
        i += m[0].length;
        continue;
      }
    }
    if (/[0-9]/.test(c) && (i === 0 || !/[\w$]/.test(code.charAt(i - 1)))) {
      const m = code.slice(i).match(numberRe);
      if (m !== null) {
        tokens.push(m[0], 'number');
        i += m[0].length;
        lineStart = false;
        continue;
      }
    }
    const id = code.slice(i).match(identRe);
    if (id !== null) {
      const word = id[0];
      const key = l.caseless ? word.toLowerCase() : word;
      const after = code.slice(i + word.length, i + word.length + 2);
      let role = 'variable';
      if (control.has(key)) {
        role = 'control';
      } else if (constants.has(word)) {
        role = 'keyword';
      } else if (l.keywords.has(key)) {
        role = 'keyword';
      } else if (langId === 'sh') {
        // A command at the start of a line reads as a function call, its arguments as plain words
        role = lineStart ? 'function' : '';
      } else if (langId === 'sql') {
        role = '';
      } else if (after.startsWith('(') || (langId === 'rust' && after.startsWith('!'))) {
        role = 'function';
      } else if (/^[A-Z]/.test(word) && langId !== 'py' || (langId === 'py' && /^[A-Z][a-z]/.test(word))) {
        role = 'type';
      }
      tokens.push(word, role);
      i += word.length;
      lineStart = false;
      continue;
    }
    // Punctuation and operators keep the card's ink; a shell flag stays one plain word
    tokens.push(c, '');
    if (!(langId === 'sh' && (c === '|' || c === ';' || c === '&'))) {
      lineStart = false;
    } else {
      lineStart = true;
    }
    i++;
  }
  return tokens.out;
}

function tokenizeJson(code: string): CodeToken[] {
  const tokens = new Tokens();
  let i = 0;
  while (i < code.length) {
    const c = code.charAt(i);
    if (c === '"') {
      const end = stringEnd(code, i, '"', false);
      // A key is a string followed by a colon
      const key = /^\s*:/.test(code.slice(end, end + 8));
      tokens.push(code.slice(i, end), key ? 'variable' : 'string');
      i = end;
      continue;
    }
    if (code.startsWith('//', i)) {
      const end = lineEnd(code, i);
      tokens.push(code.slice(i, end), 'comment');
      i = end;
      continue;
    }
    const m = code.slice(i).match(/^-?\d[\d.eE+-]*/);
    if (m !== null && (i === 0 || !/\w/.test(code.charAt(i - 1)))) {
      tokens.push(m[0], 'number');
      i += m[0].length;
      continue;
    }
    const w = code.slice(i).match(/^(true|false|null)\b/);
    if (w !== null) {
      tokens.push(w[0], 'keyword');
      i += w[0].length;
      continue;
    }
    tokens.push(c, '');
    i++;
  }
  return tokens.out;
}

/** YAML and TOML: comments, keys, strings, numbers and constants, line by line */
function tokenizeConfig(code: string, yaml: boolean): CodeToken[] {
  const tokens = new Tokens();
  const lines = code.split('\n');
  lines.forEach((line: string, n: number) => {
    let i = 0;
    const key = yaml ? line.match(/^(\s*-?\s*)([\w.\-"'/ ]+?)(:)(?=\s|$)/) : line.match(/^(\s*)([\w.\-"]+)(\s*=)/);
    const section = !yaml ? line.match(/^\s*\[[^\]]*\]/) : null;
    if (section !== null) {
      tokens.push(section[0], 'tag');
      i = section[0].length;
    } else if (key !== null) {
      tokens.push(key[1], '');
      tokens.push(key[2], yaml ? 'tag' : 'variable');
      tokens.push(key[3], '');
      i = key[0].length;
    }
    while (i < line.length) {
      const c = line.charAt(i);
      if (c === '#') {
        tokens.push(line.slice(i), 'comment');
        break;
      }
      if (c === '"' || c === "'") {
        const end = stringEnd(line, i, c, false);
        tokens.push(line.slice(i, end), 'string');
        i = end;
        continue;
      }
      const m = line.slice(i).match(/^-?\d[\d._:eE+-]*\b/);
      if (m !== null && (i === 0 || !/\w/.test(line.charAt(i - 1)))) {
        tokens.push(m[0], 'number');
        i += m[0].length;
        continue;
      }
      const w = line.slice(i).match(/^(true|false|null|yes|no|on|off)\b/);
      if (w !== null && (i === 0 || !/\w/.test(line.charAt(i - 1)))) {
        tokens.push(w[0], 'keyword');
        i += w[0].length;
        continue;
      }
      // A bare YAML value reads as a string, as dark-plus colours it
      tokens.push(c, yaml && key !== null && /\S/.test(c) ? 'string' : '');
      i++;
    }
    if (n < lines.length - 1) {
      tokens.push('\n', '');
    }
  });
  return tokens.out;
}

function tokenizeCss(code: string): CodeToken[] {
  const tokens = new Tokens();
  let i = 0;
  let inBlock = 0;
  while (i < code.length) {
    const c = code.charAt(i);
    if (code.startsWith('/*', i)) {
      const close = code.indexOf('*/', i + 2);
      const end = close < 0 ? code.length : close + 2;
      tokens.push(code.slice(i, end), 'comment');
      i = end;
      continue;
    }
    if (c === '"' || c === "'") {
      const end = stringEnd(code, i, c, false);
      tokens.push(code.slice(i, end), 'string');
      i = end;
      continue;
    }
    if (c === '{' || c === '}') {
      inBlock += c === '{' ? 1 : -1;
      tokens.push(c, '');
      i++;
      continue;
    }
    if (c === '@') {
      const m = code.slice(i).match(/^@[\w-]+/);
      const text = m !== null ? m[0] : c;
      tokens.push(text, 'control');
      i += text.length;
      continue;
    }
    if (inBlock > 0) {
      const prop = code.slice(i).match(/^[\w-]+(?=\s*:)/);
      if (prop !== null) {
        tokens.push(prop[0], 'variable');
        i += prop[0].length;
        continue;
      }
      const num = code.slice(i).match(/^(?:#[\da-fA-F]{3,8}\b|-?\d*\.?\d+(?:px|em|rem|%|vh|vw|s|ms|deg|fr)?)/);
      if (num !== null && num[0].length > 0 && (i === 0 || !/[\w-]/.test(code.charAt(i - 1)))) {
        tokens.push(num[0], 'number');
        i += num[0].length;
        continue;
      }
      const word = code.slice(i).match(/^[\w-]+/);
      if (word !== null) {
        tokens.push(word[0], 'string');
        i += word[0].length;
        continue;
      }
    } else {
      const sel = code.slice(i).match(/^[.#]?[\w-]+/);
      if (sel !== null) {
        tokens.push(sel[0], sel[0].startsWith('.') || sel[0].startsWith('#') ? 'function' : 'tag');
        i += sel[0].length;
        continue;
      }
    }
    tokens.push(c, '');
    i++;
  }
  return tokens.out;
}

function tokenizeHtml(code: string): CodeToken[] {
  const tokens = new Tokens();
  let i = 0;
  while (i < code.length) {
    if (code.startsWith('<!--', i)) {
      const close = code.indexOf('-->', i + 4);
      const end = close < 0 ? code.length : close + 3;
      tokens.push(code.slice(i, end), 'comment');
      i = end;
      continue;
    }
    const open = code.slice(i).match(/^<\/?[\w:.-]+/);
    if (open !== null) {
      const slash = open[0].startsWith('</') ? 2 : 1;
      tokens.push(open[0].slice(0, slash), 'punct');
      tokens.push(open[0].slice(slash), 'tag');
      i += open[0].length;
      // Attributes up to the tag's end
      while (i < code.length && code.charAt(i) !== '>') {
        const c = code.charAt(i);
        if (c === '"' || c === "'") {
          const end = stringEnd(code, i, c, true);
          tokens.push(code.slice(i, end), 'string');
          i = end;
          continue;
        }
        const attr = code.slice(i).match(/^[\w:.@-]+/);
        if (attr !== null) {
          tokens.push(attr[0], 'variable');
          i += attr[0].length;
          continue;
        }
        tokens.push(c, c === '/' ? 'punct' : '');
        i++;
      }
      if (i < code.length) {
        tokens.push('>', 'punct');
        i++;
      }
      continue;
    }
    tokens.push(code.charAt(i), '');
    i++;
  }
  return tokens.out;
}

function tokenizeDiff(code: string): CodeToken[] {
  const tokens = new Tokens();
  const lines = code.split('\n');
  lines.forEach((line: string, n: number) => {
    const role = line.startsWith('@@') ? 'keyword' : line.startsWith('+') ? 'number' : line.startsWith('-') ? 'string' :
      (line.startsWith('diff ') || line.startsWith('index ')) ? 'comment' : '';
    tokens.push(line, role);
    if (n < lines.length - 1) {
      tokens.push('\n', '');
    }
  });
  return tokens.out;
}

/** The text's tokens cut into lines; an unknown language gives undefined (draw it plain) */
export function highlight(code: string, language: string): CodeToken[][] | undefined {
  let flat: CodeToken[];
  if (language === 'json') {
    flat = tokenizeJson(code);
  } else if (language === 'yaml' || language === 'toml') {
    flat = tokenizeConfig(code, language === 'yaml');
  } else if (language === 'css') {
    flat = tokenizeCss(code);
  } else if (language === 'html') {
    flat = tokenizeHtml(code);
  } else if (language === 'diff') {
    flat = tokenizeDiff(code);
  } else {
    const l = lang(language);
    if (l === undefined) {
      return undefined;
    }
    flat = tokenizeCode(code, l, language);
  }
  // Cut at newlines, a token spanning lines (a block comment) split into one run per line
  const lines: CodeToken[][] = [[]];
  for (const token of flat) {
    const parts = token.text.split('\n');
    parts.forEach((part: string, k: number) => {
      if (k > 0) {
        lines.push([]);
      }
      if (part.length > 0) {
        const piece = new CodeToken();
        piece.text = part;
        piece.role = token.role;
        lines[lines.length - 1].push(piece);
      }
    });
  }
  return lines;
}

const cache: Map<string, CodeToken[][]> = new Map<string, CodeToken[][]>();
const cacheCap = 400;

/** highlight, cached by language and source: a card drawn again (a patch, a scroll back) costs a lookup */
export function highlightCached(code: string, language: string): CodeToken[][] | undefined {
  if (language.length === 0) {
    return undefined;
  }
  const key = `${language}\u0000${code}`;
  const hit = cache.get(key);
  if (hit !== undefined) {
    return hit;
  }
  const lines = highlight(code, language);
  if (lines !== undefined) {
    if (cache.size >= cacheCap) {
      cache.clear();
    }
    cache.set(key, lines);
  }
  return lines;
}
