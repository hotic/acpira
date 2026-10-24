// The element picker the engine's preview proxy adds to every HTML page it serves (rust/crates/acpira-host/src/preview).
// It runs inside someone else's page: everything it draws lives in a closed shadow root that takes no pointer events, it
// listens in the capture phase only while picking, and it answers only its parent frame (the browser panel) when the
// message carries the token this script was served with. It also hands the page window.acpira.context, the web side of the
// context API (docs/context-api.md). Built to dist/picker.js (apps/vscode/build.mjs)
import { envelopeOf, PICKER_LIMITS, type PageContextOp, type PanelMsg, type PickedElement, type PickedRect, type PickerMsg } from '@acpira/shared/picker';
import { serialize } from './serialize';

// How long a page's context call waits for the shell before its promise gives up
const CONTEXT_ANSWER_MS = 10_000;

(() => {
  const w = window as Window & { __acpiraPicker?: true; acpira?: unknown };
  // A page that pulls the script in twice (an injected document.write, an SPA re-rendering <head>) keeps the first
  if (w.__acpiraPicker || window.parent === window) return;
  w.__acpiraPicker = true;
  const token = new URL((document.currentScript as HTMLScriptElement | null)?.src ?? location.href).searchParams.get('t') ?? '';
  if (!token) return;

  const send = (msg: PickerMsg) => window.parent.postMessage({ acpiraPicker: token, msg }, '*');

  // The page's own way into the chat (docs/context-api.md, "Web pages"): window.acpira.context.set / add take the payload of
  // the commands with the same names, and the shell answers through the panel. It exists only in a page the browser panel
  // shows, set before the page's own scripts run, so a page checks for it and falls back without it
  const pageId = Math.random().toString(36).slice(2, 10);
  let calls = 0;
  const waiting = new Map<string, { op: PageContextOp; resolve: (v: { shown: boolean } | undefined) => void; reject: (e: Error) => void; timer: number }>();
  // The live slots this document filled, by source id, kept as the clears that empty them: leaving the page (a reload, a
  // link, a form) sends them, so the next page does not inherit a selection it never made
  const live = new Map<string, { source: { id: string; name: string }; owner?: string; items: [] }>();
  const track = (payload: unknown) => {
    const p = payload as { source?: { id?: unknown; name?: unknown }; owner?: unknown; items?: unknown } | null | undefined;
    const id = p?.source?.id, name = p?.source?.name;
    if (typeof id !== 'string' || typeof name !== 'string') return;
    if (!Array.isArray(p!.items) || !p!.items.length) { live.delete(id); return; }
    live.set(id, { source: { id, name }, ...(typeof p!.owner === 'string' ? { owner: p!.owner } : {}), items: [] });
  };
  const call = (op: PageContextOp, payload: unknown) => new Promise<{ shown: boolean } | undefined>((resolve, reject) => {
    const id = `${pageId}.${++calls}`;
    try {
      send({ type: 'context', id, op, payload });
    } catch (e) {
      // postMessage cannot clone it (a function, a DOM node): refused before it leaves the page
      reject(new TypeError(`acpira.context: the payload must be plain data (${e instanceof Error ? e.message : String(e)})`));
      return;
    }
    const timer = window.setTimeout(() => {
      waiting.delete(id);
      reject(new Error('acpira.context: no answer from Acpira'));
    }, CONTEXT_ANSWER_MS);
    waiting.set(id, { op, resolve, reject, timer });
    if (op === 'set') track(payload);
  });
  w.acpira = Object.freeze({
    context: Object.freeze({
      version: 1,
      set: (payload: unknown) => call('set', payload),
      add: (payload: unknown) => call('add', payload),
    }),
  });
  window.addEventListener('pagehide', () => {
    for (const clear of live.values()) {
      try { send({ type: 'context', id: `${pageId}.${++calls}`, op: 'set', payload: clear }); } catch { /* the panel went first */ }
    }
    live.clear();
  });

  // Overlay: a fixed, full-viewport host above the page
  let root: ShadowRoot | undefined;
  let overlayHost: HTMLElement | undefined;
  const paintAccent = () => {
    overlayHost?.style.setProperty('--a', accent);
    overlayHost?.style.setProperty('--f', accentFg);
  };
  let hoverBox: HTMLDivElement | undefined;
  let marquee: HTMLDivElement | undefined;
  let picked: Element[] = [];
  let pickedBoxes: HTMLDivElement[] = [];
  // Until the panel sends its own accent with `pick`
  let accent = '#3b82f6';
  let accentFg = '#ffffff';

  function overlay(): ShadowRoot {
    if (root) return root;
    const host = document.createElement('acpira-picker');
    host.style.cssText = 'all:initial;position:fixed;inset:0;z-index:2147483647;pointer-events:none;';
    overlayHost = host;
    paintAccent();
    root = host.attachShadow({ mode: 'closed' });
    const style = document.createElement('style');
    // Outlines in the panel's accent, fills faint enough to keep the page's own colours readable underneath
    style.textContent = `.box{position:fixed;box-sizing:border-box;pointer-events:none;border:1.5px solid var(--a);border-radius:4px;transition:left .08s,top .08s,width .08s,height .08s}
.hover{background:color-mix(in srgb,var(--a) 10%,transparent)}
.picked{border-width:2px;border-radius:2px}
.marquee{border-style:dashed;background:color-mix(in srgb,var(--a) 5%,transparent)}`;
    root.append(style);
    (document.documentElement ?? document.body).append(host);
    return root;
  }

  function box(cls: string): HTMLDivElement {
    const d = document.createElement('div');
    d.className = `box ${cls}`;
    overlay().append(d);
    return d;
  }

  function place(d: HTMLDivElement, r: PickedRect) {
    d.style.left = `${r.x}px`;
    d.style.top = `${r.y}px`;
    d.style.width = `${r.width}px`;
    d.style.height = `${r.height}px`;
  }

  const rectOf = (el: Element): PickedRect => {
    const r = el.getBoundingClientRect();
    return { x: r.left, y: r.top, width: r.width, height: r.height };
  };

  function labelOf(el: Element): string {
    const tag = el.tagName.toLowerCase();
    const id = el.id ? `#${el.id}` : '';
    const classes = [...el.classList].slice(0, 2).map(c => `.${c}`).join('');
    return `${tag}${id}${classes}`.slice(0, 80);
  }

  // From the nearest ancestor with a unique id (or <body>) down, :nth-of-type where a tag has siblings of its kind
  function selectorOf(el: Element): string {
    const steps: string[] = [];
    for (let e: Element | null = el; e && e !== document.documentElement; e = e.parentElement) {
      const tag = e.tagName.toLowerCase();
      if (e.id && document.querySelectorAll(`#${CSS.escape(e.id)}`).length === 1) { steps.unshift(`#${CSS.escape(e.id)}`); break; }
      if (tag === 'body') { steps.unshift('body'); break; }
      const same = e.parentElement ? [...e.parentElement.children].filter(c => c.tagName === e!.tagName) : [];
      steps.unshift(same.length > 1 ? `${tag}:nth-of-type(${same.indexOf(e) + 1})` : tag);
    }
    return steps.join(' > ');
  }

  function describe(el: Element): PickedElement {
    const text = ((el as HTMLElement).innerText ?? el.textContent ?? '').replace(/\s+/g, ' ').trim();
    return {
      label: labelOf(el),
      selector: selectorOf(el),
      text: text.length > PICKER_LIMITS.text ? `${text.slice(0, PICKER_LIMITS.text)}…` : text,
      html: serialize(el),
      rect: rectOf(el),
    };
  }

  function drawPicked() {
    for (const d of pickedBoxes) d.remove();
    pickedBoxes = picked.map(el => {
      // The outline alone: the chip and the prompt carry the selector
      const d = box('picked');
      place(d, rectOf(el));
      return d;
    });
  }

  function setPicked(next: Element[]) {
    picked = next.slice(0, PICKER_LIMITS.elements);
    hoverBox?.remove();
    hoverBox = undefined;
    drawPicked();
    send({ type: 'picked', elements: picked.map(describe) });
  }

  // Scroll and resize move the picked boxes (here) and the panel's comment card (there), once a frame
  let queued = false;
  function follow() {
    if (queued || !picked.length) return;
    queued = true;
    requestAnimationFrame(() => {
      queued = false;
      const rects = picked.map(rectOf);
      pickedBoxes.forEach((d, i) => place(d, rects[i]!));
      send({ type: 'rects', rects });
    });
  }

  // The element under the pointer, never our own overlay (it takes no pointer events, so elementFromPoint skips it)
  const under = (x: number, y: number) => {
    const el = document.elementFromPoint(x, y);
    return el && el !== document.documentElement && el.tagName.toLowerCase() !== 'acpira-picker' ? el : undefined;
  };

  // The outermost elements lying wholly inside the marquee: a picked card does not also list its own children
  function inside(r: PickedRect): Element[] {
    const out: Element[] = [];
    const visit = (el: Element) => {
      if (out.length >= PICKER_LIMITS.elements) return;
      const b = el.getBoundingClientRect();
      if (b.width === 0 && b.height === 0) return;
      const within = b.left >= r.x && b.top >= r.y && b.right <= r.x + r.width && b.bottom <= r.y + r.height;
      if (within) { out.push(el); return; }
      const overlaps = b.right > r.x && b.left < r.x + r.width && b.bottom > r.y && b.top < r.y + r.height;
      if (overlaps || getComputedStyle(el).display === 'contents') for (const c of el.children) visit(c);
    };
    if (document.body) for (const c of document.body.children) visit(c);
    return out;
  }

  let picking = false;
  let drag: { x: number; y: number; moved: boolean } | undefined;
  let lastClick: { el: Element; at: number } | undefined;
  const DRAG_PX = 4;
  const DOUBLE_MS = 400;

  const swallow = (e: Event) => {
    e.preventDefault();
    e.stopImmediatePropagation();
  };

  function onMove(e: PointerEvent) {
    swallow(e);
    if (drag) {
      if (!drag.moved && Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < DRAG_PX) return;
      drag.moved = true;
      hoverBox?.remove();
      hoverBox = undefined;
      marquee ??= box('marquee');
      place(marquee, { x: Math.min(drag.x, e.clientX), y: Math.min(drag.y, e.clientY), width: Math.abs(e.clientX - drag.x), height: Math.abs(e.clientY - drag.y) });
      return;
    }
    const el = under(e.clientX, e.clientY);
    // A picked element shows its own outline only
    if (!el || picked.includes(el)) { hoverBox?.remove(); hoverBox = undefined; return; }
    hoverBox ??= box('hover');
    place(hoverBox, rectOf(el));
  }

  function onDown(e: PointerEvent) {
    swallow(e);
    if (e.button !== 0) return;
    drag = { x: e.clientX, y: e.clientY, moved: false };
  }

  function onUp(e: PointerEvent) {
    swallow(e);
    const d = drag;
    drag = undefined;
    if (!d) return;
    if (d.moved) {
      marquee?.remove();
      marquee = undefined;
      const r = { x: Math.min(d.x, e.clientX), y: Math.min(d.y, e.clientY), width: Math.abs(e.clientX - d.x), height: Math.abs(e.clientY - d.y) };
      const found = inside(r);
      setPicked(adding(e) ? [...picked, ...found.filter(f => !picked.includes(f))] : found);
      return;
    }
    const el = under(e.clientX, e.clientY);
    if (!el) return;
    // The second click of a double-click leaves the pick alone: the double-click itself adds it (onDouble)
    const now = Date.now();
    const repeat = lastClick?.el === el && now - lastClick.at < DOUBLE_MS;
    lastClick = { el, at: now };
    if (repeat) return;
    if (!adding(e)) { setPicked([el]); return; }
    setPicked(picked.includes(el) ? picked.filter(p => p !== el) : [...picked, el]);
  }

  // Shift (design tools) or Ctrl / Cmd (browsers, file managers) adds to the pick instead of replacing it
  const adding = (e: MouseEvent) => e.shiftKey || e.ctrlKey || e.metaKey;

  function onDouble(e: MouseEvent) {
    swallow(e);
    if (picked.length) send({ type: 'add' });
  }

  function onKey(e: KeyboardEvent) {
    if (e.isComposing) return;
    if (e.key === 'Escape') { swallow(e); send({ type: 'escape' }); return; }
    if (!picked.length) return;
    if (e.key === 'Enter') { swallow(e); send({ type: 'add' }); return; }
    if (e.key.toLowerCase() === 'j' && (e.metaKey || e.ctrlKey)) { swallow(e); send({ type: 'comment' }); }
  }

  // While picking the page sees no pointer input at all: links do not navigate, buttons do not fire
  const CAPTURE: [string, (e: never) => void][] = [
    ['pointermove', onMove], ['pointerdown', onDown], ['pointerup', onUp],
    ['mousedown', swallow], ['mouseup', swallow], ['click', swallow], ['dblclick', onDouble], ['auxclick', swallow], ['contextmenu', swallow],
    ['keydown', onKey],
  ];

  function setPicking(on: boolean) {
    if (on === picking) return;
    picking = on;
    for (const [type, fn] of CAPTURE) (on ? window.addEventListener : window.removeEventListener).call(window, type, fn as EventListener, true);
    document.documentElement.style.cursor = on ? 'crosshair' : '';
    if (!on) { hoverBox?.remove(); hoverBox = undefined; marquee?.remove(); marquee = undefined; drag = undefined; }
  }

  window.addEventListener('scroll', follow, { capture: true, passive: true });
  window.addEventListener('resize', follow, { passive: true });

  window.addEventListener('message', e => {
    if (e.source !== window.parent) return;
    const msg = envelopeOf<PanelMsg>(e.data, token);
    if (!msg) return;
    switch (msg.type) {
      case 'pick':
        if (msg.accent) accent = msg.accent;
        if (msg.accentFg) accentFg = msg.accentFg;
        paintAccent();
        setPicking(msg.on);
        break;
      case 'clear': setPicked([]); break;
      case 'history': history.go(msg.go); break;
      case 'reload': location.reload(); break;
      case 'contextDone': {
        const pending = waiting.get(msg.id);
        if (!pending) break;
        waiting.delete(msg.id);
        window.clearTimeout(pending.timer);
        if (msg.error) pending.reject(new Error(msg.error));
        else pending.resolve(pending.op === 'set' ? { shown: msg.shown !== false } : undefined);
        break;
      }
    }
  });

  // Where the page is, for the panel's address bar: at load and whenever an SPA moves (history API, hash, back / forward)
  let last = '';
  const where = (type: 'hello' | 'location') => {
    const url = location.href;
    if (type === 'location' && url === last) return;
    last = url;
    send({ type, url, title: document.title });
  };
  for (const name of ['pushState', 'replaceState'] as const) {
    const orig = history[name];
    history[name] = function (this: History, ...args: Parameters<History['pushState']>) {
      const r = orig.apply(this, args);
      where('location');
      return r;
    };
  }
  window.addEventListener('popstate', () => where('location'));
  window.addEventListener('hashchange', () => where('location'));
  // The title is set late by most apps; one more report once the document is in
  const hello = () => where('hello');
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', () => { hello(); }, { once: true });
  else hello();
  window.addEventListener('load', () => send({ type: 'location', url: location.href, title: document.title }), { once: true });
})();
