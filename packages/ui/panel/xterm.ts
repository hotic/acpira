import { Terminal, type IBufferLine, type IBufferCell, type ILinkProvider, type ITheme } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebglAddon } from '@xterm/addon-webgl';
import { Unicode11Addon } from '@xterm/addon-unicode11';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { SearchAddon, type ISearchOptions } from '@xterm/addon-search';
import { Base64, ClipboardAddon } from '@xterm/addon-clipboard';
import { t } from '../i18n';
import { macOS } from '../desktop';

// The terminal tabs' front end: xterm.js, the terminal VS Code runs, with its addons for fitting the frame, the GPU
// renderer, wide characters (Unicode 11), web and file links, search and OSC 52 clipboard writes. Loaded with the first
// terminal tab (panel/terminals.ts), so the main bundle carries none of it; the engine runs the shell (terminal.rs)

export interface ViewHooks {
  // Keys and pastes; `binary`: a string of bytes (legacy mouse reports), each character one byte
  input(data: string, binary?: boolean): void;
  resize(cols: number, rows: number): void;
  // Output rendered, in UTF-16 units, for the engine's flow control
  ack(chars: number): void;
  // ⌘-click (Ctrl elsewhere) on an address, or on a path as printed (relative to wherever the program ran)
  openUrl(url: string): void;
  openPath(path: string, line?: number): void;
  // ⌘F / Ctrl+Shift+F: the frame's find bar
  find(): void;
}

// Rendered output is acknowledged in steps this size (VS Code's), well under the engine's limit
const ACK_EVERY = 5_000;

const ANSI = { black: 'brightBlack', red: 'brightRed', green: 'brightGreen', yellow: 'brightYellow', blue: 'brightBlue', magenta: 'brightMagenta', cyan: 'brightCyan', white: 'brightWhite' } as const;

let paint: CanvasRenderingContext2D | null | undefined;

// A CSS colour as #rrggbb, what the search addon's decorations take
function hex(css: string): string {
  paint ??= Object.assign(document.createElement('canvas'), { width: 1, height: 1 }).getContext('2d', { willReadFrequently: true });
  if (!paint) return '#808080';
  paint.clearRect(0, 0, 1, 1);
  paint.fillStyle = '#000';
  paint.fillStyle = css;
  paint.fillRect(0, 0, 1, 1);
  return `#${[...paint.getImageData(0, 0, 1, 1).data.slice(0, 3)].map(v => v.toString(16).padStart(2, '0')).join('')}`;
}

interface Look { theme: ITheme; fontFamily: string; fontSize: number; decorations: NonNullable<ISearchOptions['decorations']> }

// The --term-* tokens (styles/terminal.css) resolved where the terminal sits: xterm takes colours, not variables
function readLook(host: HTMLElement): Look {
  const probe = document.createElement('span');
  probe.hidden = true;
  host.appendChild(probe);
  const color = (name: string) => {
    probe.style.color = `var(--term-${name})`;
    return getComputedStyle(probe).color;
  };
  const theme: ITheme = {
    foreground: color('fg'),
    background: color('bg'),
    cursor: color('cursor'),
    cursorAccent: color('bg'),
    selectionBackground: color('selection'),
    selectionInactiveBackground: color('selection-inactive'),
    scrollbarSliderBackground: color('scrollbar'),
    scrollbarSliderHoverBackground: color('scrollbar-hover'),
    scrollbarSliderActiveBackground: color('scrollbar-active'),
  };
  for (const [name, bright] of Object.entries(ANSI) as [keyof typeof ANSI, (typeof ANSI)[keyof typeof ANSI]][]) {
    // As #rrggbb: the tokens are colour mixes, which compute to oklab()
    theme[name] = hex(color(name));
    theme[bright] = hex(color(`bright-${name}`));
  }
  const decorations = {
    matchBackground: hex(color('match')),
    matchOverviewRuler: hex(color('match')),
    activeMatchBackground: hex(color('match-active')),
    activeMatchColorOverviewRuler: hex(color('match-active')),
  };
  probe.remove();
  const style = getComputedStyle(host);
  return {
    theme,
    fontFamily: style.getPropertyValue('--font-mono').trim() || 'monospace',
    fontSize: Number.parseFloat(style.getPropertyValue('--code-font-size')) || 12,
    decorations,
  };
}

// A path a program printed: with a folder in it, or a file name with a line (`src/a.ts:12:5`, `./x.rs`, `a.ts(3,4)`,
// `/abs/file.txt`). A bare word with a dot (`node.js`, `e.g.`) is no link, nor anything inside a web address
const PATH = /(?<![\w./~-])((?:~|\.{1,2})?\/?(?:[\w@+-][\w.@+-]*\/)*[\w@+-][\w.@+-]*)(?::(\d+)(?::\d+)?|\((\d+)(?:,\s?\d+)?\))?/g;
const URL_RUN = /\b[a-z][\w+.-]*:\/\/\S+/gi;

export function pathLinksIn(text: string): { start: number; end: number; path: string; line?: number }[] {
  const urls = [...text.matchAll(URL_RUN)].map(m => [m.index, m.index + m[0].length] as const);
  const out: { start: number; end: number; path: string; line?: number }[] = [];
  for (const m of text.matchAll(PATH)) {
    const path = m[1]!;
    const lineText = m[2] ?? m[3];
    const last = path.slice(path.lastIndexOf('/') + 1);
    const ok = path.includes('/')
      ? last.includes('.') || /^(~|\.{1,2})?\/./.test(path)
      : !!lineText && last.includes('.') && !/^\d/.test(path);
    if (!ok || urls.some(([a, b]) => m.index < b && m.index + m[0].length > a)) continue;
    out.push({ start: m.index, end: m.index + m[0].length, path, ...(lineText ? { line: Number(lineText) } : {}) });
  }
  return out;
}

// A buffer line as text, with the cell each character starts in (wide characters take two cells, one character)
function cellsOf(line: IBufferLine): { text: string; at: number[] } {
  let text = '';
  const at: number[] = [];
  let cell: IBufferCell | undefined;
  for (let x = 0; x < line.length; x++) {
    cell = line.getCell(x, cell);
    if (!cell) break;
    if (cell.getWidth() === 0) continue;
    const chars = cell.getChars() || ' ';
    for (let i = 0; i < chars.length; i++) at.push(x);
    text += chars;
  }
  return { text, at };
}

export class XtermView {
  readonly term: Terminal;
  private readonly fitter = new FitAddon();
  private readonly searcher = new SearchAddon({ highlightLimit: 1000 });
  private webgl?: WebglAddon;
  private host?: HTMLElement;
  private resizes?: ResizeObserver;
  private shellChanges?: MutationObserver;
  private frame = 0;
  private styleFrame = 0;
  private unacked = 0;
  private theme?: ITheme;
  private decorations?: Look['decorations'];
  // The find bar is open (see `find`)
  private finding = false;
  private laid!: () => void;
  // The first layout in a visible frame: its size is what the engine's shell should start at
  readonly laidOut = new Promise<void>(resolve => { this.laid = resolve; });
  fitted = false;

  constructor(private readonly hooks: ViewHooks) {
    // A modifier click follows a link, a plain click selects (VS Code's default); the hint says so on hover
    const follow = (e: MouseEvent, go: () => void) => { if (macOS ? e.metaKey : e.ctrlKey) go(); };
    const hint = (on: boolean) => {
      if (this.term.element) this.term.element.title = on ? t('terminal.followLink', { key: macOS ? '⌘' : 'Ctrl' }) : '';
    };
    this.term = new Terminal({
      allowProposedApi: true,
      scrollback: 5000,
      cursorBlink: true,
      cursorStyle: 'block',
      cursorInactiveStyle: 'outline',
      lineHeight: 1.2,
      // VS Code's default: colours a theme gets wrong for the background are lifted to readable
      minimumContrastRatio: 4.5,
      rightClickSelectsWord: macOS,
      macOptionClickForcesSelection: true,
      smoothScrollDuration: 0,
      // OSC 8 hyperlinks (`ls --hyperlink`, compilers): web addresses and files only
      linkHandler: {
        allowNonHttpProtocols: true,
        activate: (e, uri) => follow(e, () => {
          if (/^https?:/i.test(uri)) hooks.openUrl(uri);
          else if (/^file:/i.test(uri)) { try { hooks.openPath(decodeURIComponent(new URL(uri).pathname)); } catch { /* not a path */ } }
        }),
        hover: () => hint(true),
        leave: () => hint(false),
      },
    });
    this.term.loadAddon(this.fitter);
    this.term.loadAddon(this.searcher);
    this.term.loadAddon(new Unicode11Addon());
    this.term.unicode.activeVersion = '11';
    this.term.loadAddon(new WebLinksAddon((e, uri) => follow(e, () => hooks.openUrl(uri)), { hover: () => hint(true), leave: () => hint(false) }));
    // OSC 52: a program may put text on the clipboard (vim, tmux over ssh), never read it back
    this.term.loadAddon(new ClipboardAddon(new Base64(), { readText: () => '', writeText: (_selection, text) => navigator.clipboard.writeText(text) }));
    this.term.registerLinkProvider(this.pathLinks(follow, hint));
    this.term.onData(data => hooks.input(data));
    this.term.onBinary(data => hooks.input(data, true));
    this.term.onResize(({ cols, rows }) => hooks.resize(cols, rows));
    this.term.attachCustomKeyEventHandler(e => this.key(e));
  }

  private pathLinks(follow: (e: MouseEvent, go: () => void) => void, hint: (on: boolean) => void): ILinkProvider {
    return {
      provideLinks: (y, callback) => {
        const line = this.term.buffer.active.getLine(y - 1);
        if (!line) return callback(undefined);
        const { text, at } = cellsOf(line);
        const links = pathLinksIn(text).map(l => ({
          range: { start: { x: at[l.start]! + 1, y }, end: { x: at[l.end - 1]! + 1, y } },
          text: text.slice(l.start, l.end),
          activate: (e: MouseEvent) => follow(e, () => this.hooks.openPath(l.path, l.line)),
          hover: () => hint(true),
          leave: () => hint(false),
        }));
        callback(links.length ? links : undefined);
      },
    };
  }

  // Output from the engine, acknowledged once rendered
  write(data: string) {
    this.term.write(data, () => {
      this.unacked += data.length;
      if (this.unacked >= ACK_EVERY) {
        this.hooks.ack(this.unacked);
        this.unacked = 0;
      }
    });
  }

  // The engine's output so far, in place of what the view had: a page that came back, or a restart. Until the frame laid
  // the view out it takes the engine's size, so the output wraps where the shell wrote it
  replay(data: string, size?: { cols: number; rows: number }) {
    this.term.reset();
    this.unacked = 0;
    if (size && !this.fitted) this.term.resize(size.cols, size.rows);
    if (data) this.term.write(data);
  }

  // A note of the page's own (the exit, a failed start), dim, on a line of its own
  note(text: string) {
    this.term.write(`\r\n\x1b[2m${text}\x1b[22m\r\n`);
  }

  // The frame the view shows in. It opens there once the frame has a size (a hidden tab measures nothing), and moves
  // with the tab: from the tool panel to the files layout's stage, or to another session's panel
  attach(host: HTMLElement) {
    this.host = host;
    this.resizes?.disconnect();
    this.resizes = new ResizeObserver(() => this.schedule());
    this.resizes.observe(host);
    // The theme, the code font size, the surface: the shell's attributes and inline variables
    this.shellChanges?.disconnect();
    const shell = host.closest('.acp-shell');
    if (shell) {
      this.shellChanges = new MutationObserver(() => {
        cancelAnimationFrame(this.styleFrame);
        this.styleFrame = requestAnimationFrame(() => this.restyle());
      });
      this.shellChanges.observe(shell, { attributes: true });
    }
    this.schedule();
  }

  detach(host: HTMLElement) {
    if (this.host !== host) return;
    this.resizes?.disconnect();
    this.shellChanges?.disconnect();
    this.resizes = this.shellChanges = this.host = undefined;
  }

  private schedule() {
    cancelAnimationFrame(this.frame);
    this.frame = requestAnimationFrame(() => this.layout());
  }

  private layout() {
    const host = this.host;
    if (!host?.isConnected || host.clientWidth === 0 || host.clientHeight === 0) return;
    const element = this.term.element;
    if (!element) {
      this.restyle();
      this.term.open(host);
      this.loadWebgl();
    } else if (element.parentElement !== host) {
      host.appendChild(element);
      this.restyle();
      this.term.refresh(0, this.term.rows - 1);
    }
    this.fitter.fit();
    this.fitted = true;
    this.laid();
  }

  private loadWebgl() {
    try {
      const gl = new WebglAddon();
      // Too many terminals, or the GPU went away: the DOM renderer takes over
      gl.onContextLoss(() => { gl.dispose(); this.webgl = undefined; });
      this.term.loadAddon(gl);
      this.webgl = gl;
    } catch {
      this.webgl = undefined;
    }
  }

  restyle() {
    if (!this.host) return;
    const look = readLook(this.host);
    this.theme = look.theme;
    this.decorations = look.decorations;
    this.applyTheme();
    if (this.term.options.fontFamily !== look.fontFamily) this.term.options.fontFamily = look.fontFamily;
    if (this.term.options.fontSize !== look.fontSize) this.term.options.fontSize = look.fontSize;
    if (this.term.element && this.host.clientWidth > 0) this.fitter.fit();
  }

  // The search addon marks the current match by selecting it, and a selection paints over decorations; with the find
  // bar's field focused that is the dim inactive selection. While the bar is open the inactive selection takes the
  // current match's colour, so the match stands out from the others; a click into the terminal shows it as a selection
  private applyTheme() {
    if (!this.theme) return;
    const active = this.finding ? this.decorations?.activeMatchBackground : undefined;
    this.term.options.theme = active ? { ...this.theme, selectionInactiveBackground: active } : this.theme;
  }

  // macOS keys as its terminals take them (VS Code sends the same), the find bar, select all; elsewhere Ctrl+Shift for
  // copy and paste, since Ctrl+C interrupts. ⌥←/→ and ⌥⌫ xterm already sends as word moves
  private key(e: KeyboardEvent): boolean {
    if (e.type !== 'keydown' || e.isComposing) return true;
    const take = () => { e.preventDefault(); e.stopPropagation(); return false; };
    const send = (seq: string) => { this.hooks.input(seq); return take(); };
    if (macOS && e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
      if (e.key === 'ArrowLeft') return send('\x01');
      if (e.key === 'ArrowRight') return send('\x05');
      if (e.key === 'Backspace') return send('\x15');
    }
    const primary = macOS ? e.metaKey && !e.ctrlKey && !e.shiftKey : e.ctrlKey && e.shiftKey && !e.metaKey;
    if (primary && !e.altKey) {
      if (e.code === 'KeyF') { this.hooks.find(); return take(); }
      if (e.code === 'KeyA') { this.term.selectAll(); return take(); }
      if (!macOS && e.code === 'KeyC') { void this.copy(); return take(); }
      if (!macOS && e.code === 'KeyV') { void this.paste(); return take(); }
    }
    return true;
  }

  hasSelection() { return this.term.hasSelection(); }
  async copy() { if (this.term.hasSelection()) await navigator.clipboard.writeText(this.term.getSelection()); }
  async paste() {
    const text = await navigator.clipboard.readText().catch(() => '');
    if (text) this.term.paste(text);
  }
  selectAll() { this.term.selectAll(); }
  clear() { this.term.clear(); }
  // A view that has not laid out yet has nothing to focus (xterm opens in the frame's first sized layout, a frame after
  // the view is made): the focus waits for it
  focus() {
    if (this.term.element) this.term.focus();
    else void this.laidOut.then(() => this.term.focus());
  }

  // The find bar: every match marked, the current one stronger; `onResults` hears where it is among them
  find(query: string, opts: { back?: boolean; incremental?: boolean } = {}) {
    if (!this.finding) {
      this.finding = true;
      this.applyTheme();
    }
    // An emptied field drops the marks and the match it had selected
    if (!query) { this.term.clearSelection(); this.searcher.clearDecorations(); return; }
    const options: ISearchOptions = { incremental: opts.incremental, ...(this.decorations ? { decorations: this.decorations } : {}) };
    if (opts.back) this.searcher.findPrevious(query, options);
    else this.searcher.findNext(query, options);
  }
  endFind() {
    this.searcher.clearDecorations();
    if (!this.finding) return;
    this.finding = false;
    this.applyTheme();
  }
  onResults(listener: (index: number, count: number) => void) {
    const sub = this.searcher.onDidChangeResults(r => listener(r.resultIndex, r.resultCount));
    return () => sub.dispose();
  }

  dispose() {
    cancelAnimationFrame(this.frame);
    cancelAnimationFrame(this.styleFrame);
    this.resizes?.disconnect();
    this.shellChanges?.disconnect();
    this.webgl?.dispose();
    this.term.dispose();
  }
}
