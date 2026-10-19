// The element picker inside a previewed page (src/picker, served by the engine's preview proxy) and the browser panel
// around it talk over postMessage. Every message carries the proxy's token: the panel ignores anything else, the picker
// only obeys its parent. The token binds a message to this page; it does not make the page's content trustworthy

export interface PickedRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PickedElement {
  // `h1#title.post-title`: the chip / list label
  label: string;
  // A CSS path from the nearest id (or the body) down to it
  selector: string;
  // Visible text, trimmed and capped
  text: string;
  // outerHTML with secrets redacted, scripts and styles emptied, capped in depth and length
  html: string;
  rect: PickedRect;
}

// Picker → panel
export type PickerMsg =
  | { type: 'hello'; url: string; title: string }
  // The page moved (history, hash) without reloading the frame
  | { type: 'location'; url: string; title: string }
  // The current selection, replacing the previous one; empty = cleared
  | { type: 'picked'; elements: PickedElement[] }
  // Where the selected elements sit now (scroll, resize), in frame coordinates
  | { type: 'rects'; rects: PickedRect[] }
  // Keys pressed in the page while picking: Escape leaves picking, Enter adds the pick, Cmd / Ctrl+J comments on it
  | { type: 'escape' }
  | { type: 'add' }
  | { type: 'comment' };

// Panel → picker
export type PanelMsg =
  // `accent` / `accentFg`: the panel's accent colours (any CSS colour), so the outlines match the chat rather than the page
  | { type: 'pick'; on: boolean; accent?: string; accentFg?: string }
  | { type: 'clear' }
  | { type: 'history'; go: -1 | 1 }
  | { type: 'reload' };

// What goes over postMessage either way: the token names the proxy that served the page, `acpiraPicker` keeps the page's
// own postMessage traffic apart
export interface PickerEnvelope<M> {
  acpiraPicker: string;
  msg: M;
}

export function envelopeOf<M>(data: unknown, token: string): M | undefined {
  const e = data as Partial<PickerEnvelope<M>> | null;
  return e && typeof e === 'object' && e.acpiraPicker === token && e.msg && typeof e.msg === 'object' ? e.msg : undefined;
}

export const PICKER_LIMITS = { elements: 20, text: 500, html: 4000, depth: 4, children: 12 } as const;

// Attribute names that tend to carry credentials; their values never leave the page
const SECRET_NAME = /token|secret|passw|pwd|api[-_]?key|apikey|auth|session|csrf|xsrf|nonce|signature|credential|cookie|private/i;
const URL_ATTRS = new Set(['href', 'src', 'action', 'formaction', 'poster', 'srcset', 'data', 'ping']);
const LONG_BASE64 = /[A-Za-z0-9+/_-]{120,}={0,2}/g;

export function isSecretName(name: string): boolean {
  return SECRET_NAME.test(name);
}

// Query parameters with secret-looking names keep their name, lose their value; data: URLs keep only their type
export function redactUrl(value: string): string {
  const v = value.trim();
  if (/^data:/i.test(v)) return `${v.slice(0, v.indexOf(',') + 1 || 30)}[data]`;
  const q = v.indexOf('?');
  if (q < 0) return scrubText(v);
  const hash = v.indexOf('#', q);
  const query = v.slice(q + 1, hash < 0 ? undefined : hash);
  const kept = query.split('&').map(pair => {
    const eq = pair.indexOf('=');
    const name = eq < 0 ? pair : pair.slice(0, eq);
    return eq >= 0 && isSecretName(decodeSafe(name)) ? `${name}=[redacted]` : pair;
  }).join('&');
  return scrubText(`${v.slice(0, q + 1)}${kept}${hash < 0 ? '' : v.slice(hash)}`);
}

// One attribute as it may leave the page: undefined drops it
export function redactAttr(tag: string, type: string | undefined, name: string, value: string): string | undefined {
  const n = name.toLowerCase();
  // Event handlers are code, not content
  if (n.startsWith('on')) return undefined;
  if (n === 'value' && tag === 'input' && (type === 'password' || type === 'hidden')) return '[redacted]';
  if (isSecretName(n)) return '[redacted]';
  if (n === 'content' && tag === 'meta') return scrubText(value);
  if (URL_ATTRS.has(n)) return redactUrl(value);
  return scrubText(value);
}

// Long base64 runs (inline keys, embedded blobs) become a placeholder
export function scrubText(text: string): string {
  return text.replace(LONG_BASE64, '[base64]');
}

function decodeSafe(s: string): string {
  try { return decodeURIComponent(s); } catch { return s; }
}

// What a set of picked elements becomes in the composer: one item, its label and the text the agent reads. Markdown, since
// the attachment peek renders it: the page and each element's facts are one paragraph of hard-broken lines (two trailing
// spaces), so they keep one line spacing, and the HTML follows in a fence
export function pickedContext(elements: PickedElement[], pageUrl: string, pageTitle: string): { label: string; text: string } {
  const lines = [`Page: ${pageTitle ? `${pageTitle} ` : ''}<${pageUrl}>`];
  elements.forEach((e, i) => {
    const facts = [`Element${elements.length > 1 ? ` ${i + 1}` : ''}: \`${e.selector}\``];
    if (e.text) facts.push(`Text: ${e.text}`);
    lines.push('', facts.join('  \n'), '', '```html', e.html, '```');
  });
  const first = elements[0]?.label ?? 'element';
  return { label: elements.length > 1 ? `${first} +${elements.length - 1}` : first, text: lines.join('\n') };
}

// What the address bar or the open command was given, as a page the preview proxy takes: `5173` and `localhost:5173/a`
// become http://localhost URLs, the any-address a dev server prints (0.0.0.0, [::]) becomes localhost. Undefined for
// anything that is not a loopback http page (the engine refuses those too; this only spares the round trip)
export function normalizePreviewUrl(input: string): string | undefined {
  let s = input.trim();
  if (!s) return undefined;
  if (/^\d{1,5}(\/|$)/.test(s)) s = `localhost:${s}`;
  if (!/^[a-z][a-z0-9+.-]*:\/\//i.test(s)) s = `http://${s}`;
  let url: URL;
  try { url = new URL(s); } catch { return undefined; }
  if (url.protocol !== 'http:' || url.username || url.password) return undefined;
  if (url.hostname === '0.0.0.0' || url.hostname === '[::]') url.hostname = 'localhost';
  if (!['localhost', '127.0.0.1', '[::1]'].includes(url.hostname)) return undefined;
  return url.toString();
}
