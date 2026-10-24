import { PICKER_LIMITS, redactAttr, scrubText } from '@shared/picker';

// The few DOM members the serializer reads, so it runs (and is tested) without a DOM: a real Element fits this shape
export interface MiniNode {
  nodeType: number;
  nodeValue: string | null;
  childNodes: ArrayLike<MiniNode>;
  // Elements only
  tagName?: string;
  attributes?: ArrayLike<{ name: string; value: string }>;
}

const ELEMENT = 1;
const TEXT = 3;
// Code, styling and inert markup: kept as an empty tag so the structure still reads
const EMPTIED = new Set(['script', 'style', 'noscript', 'template', 'iframe', 'object', 'embed']);
// Vector drawings are noise to an agent at any depth below the <svg> itself
const OPAQUE = new Set(['svg', 'canvas', 'video', 'audio', 'picture', 'math']);
const VOID = new Set(['area', 'base', 'br', 'col', 'embed', 'hr', 'img', 'input', 'link', 'meta', 'source', 'track', 'wbr']);
const TEXT_NODE_MAX = 200;

const escText = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
const escAttr = (s: string) => escText(s).replace(/"/g, '&quot;');
const clip = (s: string, max: number) => (s.length > max ? `${s.slice(0, max)}…` : s);

function attrsOf(node: MiniNode, tag: string): string {
  const list = node.attributes ? Array.from(node.attributes) : [];
  const type = list.find(a => a.name.toLowerCase() === 'type')?.value.toLowerCase();
  let out = '';
  for (const a of list) {
    const v = redactAttr(tag, type, a.name, a.value);
    if (v === undefined) continue;
    out += v === '' ? ` ${a.name}` : ` ${a.name}="${escAttr(clip(v, TEXT_NODE_MAX))}"`;
  }
  return out;
}

// outerHTML as it may leave the page: attributes redacted, code emptied, text scrubbed, `depth` levels of children with at
// most `children` of each, and the whole capped at `max` characters (cut at a tag boundary where it can be)
export function serialize(node: MiniNode, limits: { depth: number; children: number; max: number } = { depth: PICKER_LIMITS.depth, children: PICKER_LIMITS.children, max: PICKER_LIMITS.html }): string {
  const parts: string[] = [];
  let size = 0;
  let full = false;
  const put = (s: string) => {
    if (full) return;
    if (size + s.length > limits.max) { full = true; parts.push('…'); return; }
    parts.push(s);
    size += s.length;
  };
  const walk = (n: MiniNode, depth: number) => {
    if (full) return;
    if (n.nodeType === TEXT) {
      const text = (n.nodeValue ?? '').replace(/\s+/g, ' ');
      if (text.trim()) put(escText(clip(scrubText(text), TEXT_NODE_MAX)));
      return;
    }
    if (n.nodeType !== ELEMENT || !n.tagName) return;
    const tag = n.tagName.toLowerCase();
    put(`<${tag}${attrsOf(n, tag)}>`);
    if (VOID.has(tag)) return;
    const kids = Array.from(n.childNodes).filter(k => k.nodeType === ELEMENT || (k.nodeType === TEXT && (k.nodeValue ?? '').trim()));
    if (EMPTIED.has(tag)) {
      // Nothing inside
    } else if (OPAQUE.has(tag) || depth >= limits.depth) {
      if (kids.length) put('…');
    } else {
      kids.slice(0, limits.children).forEach(k => walk(k, depth + 1));
      if (kids.length > limits.children) put(`<!-- ${kids.length - limits.children} more -->`);
    }
    put(`</${tag}>`);
  };
  walk(node, 0);
  return parts.join('');
}
