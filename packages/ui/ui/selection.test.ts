import { describe, expect, it, vi } from 'vitest';
import { fixStaleSelection } from './selection';

interface FakeElement {
  nodeType: 1;
  tagName: string;
  isConnected: boolean;
  parentElement: FakeElement | null;
  style: { contain: string; contentVisibility: string; textShadow: string };
  scrollHeight: number;
  clientHeight: number;
  scrollWidth: number;
  clientWidth: number;
  clientLeft: number;
  clientTop: number;
  rect: { left: number; top: number };
  matches: Set<string>;
  computed: Record<string, string>;
  descendants: FakeElement[];
  closest: (selector: string) => FakeElement | null;
  contains: (node: unknown) => boolean;
  getBoundingClientRect: () => { left: number; top: number };
  querySelectorAll: (selector: string) => FakeElement[];
}

function makeElement(opts: {
  tagName?: string;
  matches?: string[];
  parent?: FakeElement | null;
  computed?: Record<string, string>;
  rect?: { left: number; top: number };
  scrollHeight?: number;
  clientHeight?: number;
  scrollWidth?: number;
  clientWidth?: number;
} = {}): FakeElement {
  const el: FakeElement = {
    nodeType: 1,
    tagName: (opts.tagName ?? 'DIV').toUpperCase(),
    isConnected: true,
    parentElement: opts.parent ?? null,
    style: { contain: '', contentVisibility: '', textShadow: '' },
    scrollHeight: opts.scrollHeight ?? 100,
    clientHeight: opts.clientHeight ?? 100,
    scrollWidth: opts.scrollWidth ?? 100,
    clientWidth: opts.clientWidth ?? 100,
    clientLeft: 0,
    clientTop: 0,
    rect: opts.rect ?? { left: 0, top: 0 },
    matches: new Set(opts.matches ?? []),
    computed: opts.computed ?? {},
    descendants: [],
    closest(selector: string) {
      const parts = selector.split(',').map(s => s.trim());
      for (let cur: FakeElement | null = el; cur; cur = cur.parentElement) {
        if (parts.some(p => cur!.matches.has(p) || p.toUpperCase() === cur!.tagName)) return cur;
      }
      return null;
    },
    contains(node: unknown) {
      for (let cur = node as FakeElement | null; cur; cur = cur.parentElement ?? null) {
        if (cur === el) return true;
      }
      return false;
    },
    getBoundingClientRect: () => el.rect,
    querySelectorAll(selector: string) {
      const parts = selector.split(',').map(s => s.trim());
      return el.descendants.filter(d => parts.some(p => d.matches.has(p) || p.toUpperCase() === d.tagName));
    },
  };
  return el;
}

function makeHarness(webkit = true) {
  const listeners = new Map<string, Set<(e: unknown) => void>>();
  let rafSeq = 0;
  const rafs = new Map<number, FrameRequestCallback>();
  let activeElement: FakeElement | null = null;

  let selection: {
    isCollapsed: boolean;
    rangeCount: number;
    anchorNode: unknown;
    anchorOffset: number;
    focusNode: unknown;
    focusOffset: number;
    range?: {
      startContainer: unknown;
      endContainer: unknown;
      commonAncestorContainer: unknown;
      intersectsNode: (el: FakeElement) => boolean;
    };
    removeAllRanges: ReturnType<typeof vi.fn>;
  } = {
    isCollapsed: true,
    rangeCount: 0,
    anchorNode: null,
    anchorOffset: 0,
    focusNode: null,
    focusOffset: 0,
    removeAllRanges: vi.fn(),
  };

  const removeAllRanges = vi.fn(() => {
    selection.isCollapsed = true;
    selection.rangeCount = 0;
    selection.anchorNode = null;
    selection.focusNode = null;
    selection.range = undefined;
  });
  selection.removeAllRanges = removeAllRanges;

  const doc = {
    get activeElement() { return activeElement; },
    getSelection: () => ({
      ...selection,
      getRangeAt: () => selection.range,
    }),
    addEventListener(type: string, fn: (e: unknown) => void) {
      let set = listeners.get(type);
      if (!set) { set = new Set(); listeners.set(type, set); }
      set.add(fn);
    },
    removeEventListener(type: string, fn: (e: unknown) => void) {
      listeners.get(type)?.delete(fn);
    },
    defaultView: {
      getComputedStyle: (el: FakeElement) => ({
        userSelect: el.computed.userSelect ?? 'auto',
        getPropertyValue: (prop: string) => el.computed[prop] ?? '',
      }),
      requestAnimationFrame: (cb: FrameRequestCallback) => {
        const id = ++rafSeq;
        rafs.set(id, cb);
        return id;
      },
      cancelAnimationFrame: (id: number) => { rafs.delete(id); },
    },
  } as unknown as Document;

  const emit = (type: string, event: Record<string, unknown> = {}) => {
    for (const fn of [...(listeners.get(type) ?? [])]) fn(event);
  };

  const selectRange = (start: FakeElement, end: FakeElement = start, common: FakeElement = start, between: FakeElement[] = []) => {
    selection.isCollapsed = false;
    selection.rangeCount = 1;
    selection.anchorNode = start;
    selection.anchorOffset = 0;
    selection.focusNode = end;
    selection.focusOffset = 5;
    selection.range = {
      startContainer: start,
      endContainer: end,
      commonAncestorContainer: common,
      intersectsNode: el => between.includes(el) || el === start || el === end,
    };
    emit('selectionchange');
  };

  const collapseSelection = () => {
    selection.isCollapsed = true;
    selection.rangeCount = 0;
    selection.anchorNode = null;
    selection.focusNode = null;
    selection.range = undefined;
    emit('selectionchange');
  };

  const flushRaf = () => {
    const batch = [...rafs.entries()];
    rafs.clear();
    for (const [, cb] of batch) cb(16);
  };

  const dispose = fixStaleSelection(doc, { webkit });
  return {
    doc,
    emit,
    selectRange,
    collapseSelection,
    flushRaf,
    removeAllRanges,
    setActive: (el: FakeElement | null) => { activeElement = el; },
    dispose,
  };
}

describe('fixStaleSelection', () => {
  it('clears selection on primary mousedown over user-select: none chrome, buttons, and drag regions', () => {
    const h = makeHarness(false);
    const exchange = makeElement({ matches: ['[data-exchange]'] });
    const text = makeElement({ parent: exchange });
    const row = makeElement({ computed: { userSelect: 'none' } });
    const button = makeElement({ tagName: 'button' });
    const drag = makeElement({ matches: ['[data-drag-region]'] });
    const dragTop = makeElement({ matches: ['[data-drag-top]'], computed: { '--hdr': '40px' }, rect: { left: 0, top: 10 } });

    for (const [target, y] of [[row, 50], [button, 50], [drag, 20], [dragTop, 30]] as const) {
      h.removeAllRanges.mockClear();
      h.selectRange(text);
      h.emit('mousedown', { button: 0, shiftKey: false, detail: 1, clientX: 20, clientY: y, target });
      expect(h.removeAllRanges).toHaveBeenCalledTimes(1);
    }
    // Outside the title band on a data-drag-top sheet does not clear on mousedown alone
    h.removeAllRanges.mockClear();
    h.selectRange(text);
    h.emit('mousedown', { button: 0, shiftKey: false, detail: 1, clientX: 20, clientY: 80, target: dragTop });
    expect(h.removeAllRanges).not.toHaveBeenCalled();
    h.dispose();
  });

  it('preserves selection for quote toolbar, menus, right-click, shift-click, multi-click, scrollbars, and input selections', () => {
    const h = makeHarness(false);
    const text = makeElement();
    const quoteBtn = makeElement({ tagName: 'button', parent: makeElement({ matches: ['[data-quote-toolbar]'] }) });
    const appMenuBtn = makeElement({ tagName: 'button', matches: ['[data-app-menus]'] });
    const menuItem = makeElement({ matches: ['[role="menuitem"]'] });
    const scroller = makeElement({ scrollHeight: 400, clientHeight: 200, clientWidth: 100, rect: { left: 0, top: 0 } });
    const row = makeElement({ computed: { userSelect: 'none' } });

    for (const [target, extra] of [
      [quoteBtn, {}],
      [appMenuBtn, {}],
      [menuItem, {}],
      [scroller, { clientX: 105, clientY: 50 }],
      [row, { button: 2 }],
      [row, { shiftKey: true }],
      [row, { detail: 2 }],
    ] as const) {
      h.removeAllRanges.mockClear();
      h.selectRange(text);
      h.emit('mousedown', { button: 0, shiftKey: false, detail: 1, clientX: 20, clientY: 20, target, ...extra });
      h.emit('mouseup', { button: 0, shiftKey: false, detail: 1, clientX: 20, clientY: 20, target, ...extra });
      expect(h.removeAllRanges).not.toHaveBeenCalled();
    }

    // Selection mirrored from an active textarea is untouched
    const parent = makeElement();
    const textarea = makeElement({ tagName: 'textarea', parent });
    h.setActive(textarea);
    h.removeAllRanges.mockClear();
    h.selectRange(parent);
    h.emit('mousedown', { button: 0, shiftKey: false, detail: 1, clientX: 20, clientY: 20, target: row });
    expect(h.removeAllRanges).not.toHaveBeenCalled();
    h.dispose();
  });

  it('clears a lingering selection on single click elsewhere, but not after a drag', () => {
    const h = makeHarness(false);
    const text = makeElement();
    const blank = makeElement();

    // Single click on selectable area where browser failed to clear the pre-existing selection
    h.selectRange(text);
    h.emit('mousedown', { button: 0, shiftKey: false, detail: 1, clientX: 30, clientY: 30, target: blank });
    expect(h.removeAllRanges).not.toHaveBeenCalled();
    h.emit('mouseup', { button: 0, shiftKey: false, detail: 1, clientX: 32, clientY: 31, target: blank });
    expect(h.removeAllRanges).toHaveBeenCalledTimes(1);

    // Drag (> 4 px) does not clear
    h.removeAllRanges.mockClear();
    h.selectRange(text);
    h.emit('mousedown', { button: 0, shiftKey: false, detail: 1, clientX: 30, clientY: 30, target: blank });
    h.emit('mouseup', { button: 0, shiftKey: false, detail: 1, clientX: 50, clientY: 30, target: blank });
    expect(h.removeAllRanges).not.toHaveBeenCalled();

    // Native dragstart cancels the fallback
    h.removeAllRanges.mockClear();
    h.selectRange(text);
    h.emit('mousedown', { button: 0, shiftKey: false, detail: 1, clientX: 30, clientY: 30, target: text });
    h.emit('dragstart');
    h.emit('mouseup', { button: 0, shiftKey: false, detail: 1, clientX: 30, clientY: 30, target: text });
    expect(h.removeAllRanges).not.toHaveBeenCalled();
    h.dispose();
  });

  it('nudges repaint on departed contained blocks for one painted frame on WebKit only', () => {
    const h = makeHarness(true);
    const ex1 = makeElement({ matches: ['[data-exchange]'] });
    const ex2 = makeElement({ matches: ['[data-exchange]'] });
    const ex3 = makeElement({ matches: ['[data-exchange]'] });
    const thread = makeElement();
    thread.descendants = [ex1, ex2, ex3];
    const t1 = makeElement({ parent: ex1 });
    const t3 = makeElement({ parent: ex3 });

    // Multi-exchange selection covers ex1, ex2 (middle), and ex3
    h.selectRange(t1, t3, thread, [ex1, ex2, ex3]);
    expect(ex1.style.contain).toBe('');

    // Collapsing selection schedules repaint on all three exchanges
    h.collapseSelection();
    expect(ex1.style.contain).toBe('');

    // Frame 1 applies temporary repaint styles
    h.flushRaf();
    for (const ex of [ex1, ex2, ex3]) {
      expect(ex.style.contain).toBe('none');
      expect(ex.style.contentVisibility).toBe('visible');
      expect(ex.style.textShadow).toBe('0 0 0 transparent');
    }

    // Frame 2 restores original inline styles
    h.flushRaf();
    for (const ex of [ex1, ex2, ex3]) {
      expect(ex.style.contain).toBe('');
      expect(ex.style.contentVisibility).toBe('');
      expect(ex.style.textShadow).toBe('');
    }
    h.dispose();

    // Non-WebKit engines skip the repaint nudge completely
    const nonWebkit = makeHarness(false);
    const ex = makeElement({ matches: ['[data-exchange]'] });
    const child = makeElement({ parent: ex });
    nonWebkit.selectRange(child);
    nonWebkit.collapseSelection();
    nonWebkit.flushRaf();
    expect(ex.style.contain).toBe('');
    nonWebkit.dispose();
  });

  it('restores inline styles if disposed mid-repaint', () => {
    const h = makeHarness(true);
    const ex = makeElement({ matches: ['[data-exchange]'] });
    const child = makeElement({ parent: ex });
    h.selectRange(child);
    h.collapseSelection();
    h.flushRaf();
    expect(ex.style.contain).toBe('none');
    h.dispose();
    expect(ex.style.contain).toBe('');
    expect(ex.style.contentVisibility).toBe('');
    expect(ex.style.textShadow).toBe('');
  });
});
