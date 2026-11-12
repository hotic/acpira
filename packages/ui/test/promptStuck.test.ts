import { describe, expect, it } from 'vitest';
import { anchorOffset, followsBottom, pressAnchorNode, scrollerUsable } from '../chat/promptStuck';

describe('followsBottom', () => {
  it('keeps following when a late scroll event reads a gap left by a second viewport shrink', () => {
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1100, clientHeight: 840 }, true, 1040)).toBe(true);
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1100, clientHeight: 840 }, true, 1100)).toBe(true);
  });

  it('releases only on an upward scroll away from the bottom, and re-arms at the bottom', () => {
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 900, clientHeight: 900 }, true, 1100)).toBe(false);
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1080, clientHeight: 900 }, true, 1100)).toBe(false);
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1000, clientHeight: 900 }, false, 900)).toBe(false);
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1100, clientHeight: 900 }, false, 900)).toBe(true);
  });

  it('lets small trackpad steps escape streaming and does not re-arm on their trailing scroll events', () => {
    const el = { scrollHeight: 2000, scrollTop: 1080, clientHeight: 900 };
    const pinned = followsBottom(el, true, 1100);
    expect(pinned).toBe(false);
    expect(followsBottom(el, pinned, el.scrollTop)).toBe(false);
    // New content must not take back a viewport the user just moved.
    expect(followsBottom({ ...el, scrollHeight: 2034 }, pinned, el.scrollTop)).toBe(false);
    expect(followsBottom({ ...el, scrollTop: 1090 }, false, 1080)).toBe(false);
  });

  it('keeps following through an upward clamp the reader did not make', () => {
    // Content shrank for a layout and the browser clamped the offset: no wheel, touch, key or pointer behind it
    expect(followsBottom({ scrollHeight: 1254, scrollTop: 488, clientHeight: 666 }, true, 524, false)).toBe(true);
    expect(followsBottom({ scrollHeight: 1254, scrollTop: 488, clientHeight: 666 }, true, 524, true)).toBe(false);
    // A released follow stays released until the bottom, intent or not
    expect(followsBottom({ scrollHeight: 1254, scrollTop: 488, clientHeight: 666 }, false, 524, false)).toBe(false);
  });

  it('tolerates fractional bottom offsets and a layout shrink that clamps to the bottom', () => {
    expect(followsBottom({ scrollHeight: 2000, scrollTop: 1099.5, clientHeight: 900 }, false, 1090)).toBe(true);
    expect(followsBottom({ scrollHeight: 1800, scrollTop: 900, clientHeight: 900 }, true, 1100)).toBe(true);
  });

  it('releases on a downward user scroll short of the bottom after a fold was opened by hand', () => {
    // A thought opened at the bottom grows scrollHeight; scrolling down into it must not stay pinned or snap to the tail
    expect(followsBottom({ scrollHeight: 2200, scrollTop: 1120, clientHeight: 900 }, true, 1100, true, false)).toBe(false);
    expect(followsBottom({ scrollHeight: 2200, scrollTop: 1300, clientHeight: 900 }, false, 1120, true, false)).toBe(true);
  });
});

describe('scrollerUsable', () => {
  it('rejects a collapsed box and accepts a real thread', () => {
    expect(scrollerUsable({ clientHeight: 0, clientWidth: 320 })).toBe(false);
    expect(scrollerUsable({ clientHeight: 400, clientWidth: 0 })).toBe(false);
    expect(scrollerUsable({ clientHeight: 400, clientWidth: 320 })).toBe(true);
  });
});

interface MockSpec {
  tag?: string;
  classes?: string[];
  attrs?: Record<string, string>;
  overflowY?: string;
  children?: MockNode[];
}

interface MockNode {
  nodeType: 1;
  tag: string;
  classes: Set<string>;
  attrs: Record<string, string>;
  overflowY: string;
  parentElement: MockNode | null;
  children: MockNode[];
  contains(other: unknown): boolean;
  matches(selector: string): boolean;
  closest(selector: string): MockNode | null;
}

function el({ tag = 'div', classes = [], attrs = {}, overflowY = 'visible', children = [] }: MockSpec = {}): MockNode {
  const node: MockNode = {
    nodeType: 1,
    tag,
    classes: new Set(classes),
    attrs,
    overflowY,
    parentElement: null,
    children,
    contains(other: unknown) {
      for (let cur = other as MockNode | null; cur; cur = cur.parentElement) {
        if (cur === node) return true;
      }
      return false;
    },
    matches(selector: string) {
      return selector.split(',').some(part => {
        const token = part.trim();
        if (token.startsWith('.')) return node.classes.has(token.slice(1));
        const attr = /^\[([^=\]]+)(?:="([^"]*)")?\]$/.exec(token);
        if (attr) return attr[2] === undefined ? attr[1]! in node.attrs : node.attrs[attr[1]!] === attr[2];
        return node.tag === token;
      });
    },
    closest(selector: string) {
      for (let cur: MockNode | null = node; cur; cur = cur.parentElement) {
        if (cur.matches(selector)) return cur;
      }
      return null;
    },
  };
  for (const child of children) child.parentElement = node;
  return node;
}

const asEl = (node: MockNode) => node as unknown as Element;
const overflowOf = (node: Element) => (node as unknown as MockNode).overflowY;

describe('pressAnchorNode', () => {
  it('climbs from inner fold icons and shimmers to the owning trigger row', () => {
    const path = el({ tag: 'path' });
    const svg = el({ tag: 'svg', children: [path] });
    const shimmer = el({ tag: 'span', classes: ['shimmer-clip'] });
    const trigger = el({ tag: 'button', classes: ['action-row'], children: [svg, shimmer] });
    const content = el({ children: [trigger] });
    expect(pressAnchorNode(asEl(path), asEl(content), overflowOf)).toBe(trigger);
    expect(pressAnchorNode(asEl(shimmer), asEl(content), overflowOf)).toBe(trigger);
  });

  it('escapes nested scrollports so scrolling a thought body cannot move the outer thread', () => {
    const inline = el({ tag: 'code' });
    const paragraph = el({ tag: 'p', children: [inline] });
    const thoughtBody = el({ overflowY: 'auto', children: [paragraph] });
    const content = el({ children: [thoughtBody] });
    expect(pressAnchorNode(asEl(inline), asEl(content), overflowOf)).toBe(thoughtBody);
  });

  it('anchors command prompt toggles on the prompt header rather than the trailing button', () => {
    const icon = el({ tag: 'svg' });
    const toggle = el({ tag: 'button', classes: ['command-prompt-toggle'], children: [icon] });
    const prompt = el({ classes: ['command-prompt'], children: [toggle] });
    const content = el({ children: [prompt] });
    expect(pressAnchorNode(asEl(icon), asEl(content), overflowOf)).toBe(prompt);
  });

  it('ignores sticky prompt cards and targets outside the transcript content', () => {
    const promptText = el({ tag: 'span' });
    const sticky = el({ attrs: { 'data-sticky-prompt': '' }, children: [promptText] });
    const content = el({ children: [sticky] });
    const scrollbar = el();
    expect(pressAnchorNode(asEl(promptText), asEl(content), overflowOf)).toBeUndefined();
    expect(pressAnchorNode(asEl(scrollbar), asEl(content), overflowOf)).toBeUndefined();
  });
});

describe('anchorOffset', () => {
  const scroller = { getBoundingClientRect: () => ({ top: 48 }) };

  it('measures relative to the scroller top and drops detached or display:none nodes', () => {
    expect(anchorOffset({ isConnected: true, getClientRects: () => ({ length: 1 }), getBoundingClientRect: () => ({ top: 348 }) }, scroller)).toBe(300);
    expect(anchorOffset({ isConnected: false, getClientRects: () => ({ length: 1 }), getBoundingClientRect: () => ({ top: 348 }) }, scroller)).toBeUndefined();
    expect(anchorOffset({ isConnected: true, getClientRects: () => ({ length: 0 }), getBoundingClientRect: () => ({ top: 0 }) }, scroller)).toBeUndefined();
  });
});
