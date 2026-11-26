// Fixes two ways text selections stay stuck on screen after clicking elsewhere:
// 1. WebKit keeps an active document selection when the next click lands on unselectable chrome (`user-select: none` on
//    ui/Row and SessionList rows, or UA `-webkit-user-select: none` on buttons), and desktop window drag regions call
//    `preventDefault()` on `mousedown` (apps/desktop/src/init.js), blocking the native clear on every engine.
// 2. Under `contain: paint` / `content-visibility: auto` (transcript `[data-exchange]`, file viewer blocks) paired with
//    `position: sticky` prompt cards, WebKit can leave ghost blue `::selection` pixels painted on some blocks even after
//    the DOM selection has collapsed. Nudging those previously selected blocks for one painted frame forces re-rasterization
//    without altering layout or breaking `position: sticky`.

const PRESERVE = '[data-quote-toolbar], [data-app-menus], [role="menu"], [role="menuitem"], [data-preserve-selection]';
const EDITABLE = 'input, textarea, select, [contenteditable=""], [contenteditable="true"]';
const UNSELECTABLE_CONTROL = 'button, [role="button"], summary';
const CONTAINED = '[data-exchange], [data-preview-part], .code-block';
const CLICK_SLOP = 4;

interface SelectionSnapshot {
  x: number;
  y: number;
  anchorNode: Node | null;
  anchorOffset: number;
  focusNode: Node | null;
  focusOffset: number;
}

interface SavedStyle {
  contain: string;
  contentVisibility: string;
  textShadow: string;
}

export interface SelectionFixOptions {
  webkit?: boolean;
}

const isElement = (node: unknown): node is Element =>
  !!node && typeof node === 'object' && (node as Node).nodeType === 1 && typeof (node as Element).closest === 'function';

function elementOf(node: Node | null): Element | null {
  if (!node) return null;
  return isElement(node) ? node : node.parentElement;
}

// Some engines mirror an <input> / <textarea> selection onto document.getSelection() anchored on the field or its parent;
// leave native field selection handling alone.
function selectionInInput(doc: Document, sel: Selection): boolean {
  const active = doc.activeElement;
  if (!active || (active.tagName !== 'INPUT' && active.tagName !== 'TEXTAREA')) return false;
  const anchor = sel.anchorNode;
  if (!anchor) return false;
  if (anchor === active || (typeof active.contains === 'function' && active.contains(anchor))) return true;
  return active.parentElement === anchor && sel.focusNode === anchor;
}

function onScrollbar(el: Element, e: MouseEvent): boolean {
  const h = el as HTMLElement;
  if (typeof h.getBoundingClientRect !== 'function') return false;
  const rect = h.getBoundingClientRect();
  if (h.scrollHeight > h.clientHeight && e.clientX >= rect.left + (h.clientLeft || 0) + h.clientWidth) return true;
  if (h.scrollWidth > h.clientWidth && e.clientY >= rect.top + (h.clientTop || 0) + h.clientHeight) return true;
  return false;
}

function inTitleBand(el: Element, e: MouseEvent, getStyle: (target: Element) => CSSStyleDeclaration): boolean {
  const sheet = el.closest('[data-drag-top]');
  if (!sheet) return false;
  const hdr = parseFloat(getStyle(sheet).getPropertyValue('--hdr')) || 0;
  return hdr > 0 && e.clientY - sheet.getBoundingClientRect().top < hdr;
}

function isUnselectableChrome(el: Element, e: MouseEvent, getStyle: (target: Element) => CSSStyleDeclaration): boolean {
  if (el.closest(EDITABLE)) return false;
  // Window drag regions prevent default on mousedown (apps/desktop/src/init.js)
  if (el.closest('[data-drag-region]') || inTitleBand(el, e, getStyle)) return true;
  // Buttons and summary disclosures carry UA -webkit-user-select: none in WebKit
  if (el.closest(UNSELECTABLE_CONTROL)) return true;
  const style = getStyle(el);
  return style.userSelect === 'none' || style.getPropertyValue('-webkit-user-select') === 'none';
}

function sameSelection(sel: Selection, snap: SelectionSnapshot): boolean {
  return !sel.isCollapsed
    && sel.rangeCount > 0
    && sel.anchorNode === snap.anchorNode
    && sel.anchorOffset === snap.anchorOffset
    && sel.focusNode === snap.focusNode
    && sel.focusOffset === snap.focusOffset;
}

function selectedContained(sel: Selection | null): Set<HTMLElement> {
  const out = new Set<HTMLElement>();
  if (!sel || sel.isCollapsed || sel.rangeCount === 0) return out;
  for (let i = 0; i < sel.rangeCount; i++) {
    const range = sel.getRangeAt(i);
    const start = elementOf(range.startContainer)?.closest<HTMLElement>(CONTAINED);
    const end = elementOf(range.endContainer)?.closest<HTMLElement>(CONTAINED);
    if (start) out.add(start);
    if (end) out.add(end);
    // Selections spanning multiple exchanges also cover every contained block in between
    const common = elementOf(range.commonAncestorContainer);
    if (common && !common.closest(CONTAINED) && typeof common.querySelectorAll === 'function') {
      for (const el of common.querySelectorAll<HTMLElement>(CONTAINED)) {
        if (typeof range.intersectsNode === 'function' && range.intersectsNode(el)) out.add(el);
      }
    }
  }
  return out;
}

export function fixStaleSelection(doc: Document = document, options?: SelectionFixOptions): () => void {
  const view = doc.defaultView ?? globalThis;
  const getStyle = (el: Element) => (view.getComputedStyle ?? globalThis.getComputedStyle)(el);
  const raf = (fn: FrameRequestCallback) => (view.requestAnimationFrame ?? globalThis.requestAnimationFrame)(fn);
  const cancelRaf = (id: number) => (view.cancelAnimationFrame ?? globalThis.cancelAnimationFrame)(id);
  const webkit = options?.webkit ?? (typeof CSS !== 'undefined' && typeof CSS.supports === 'function' && CSS.supports('-webkit-nbsp-mode', 'space'));

  let pending: SelectionSnapshot | undefined;
  let tracked = new Set<HTMLElement>();

  // Repaint nudge state (WebKit only): apply in frame N, restore in frame N+1 so WebKit actually composites one clean frame
  const queued = new Set<HTMLElement>();
  const saved = new Map<HTMLElement, SavedStyle>();
  let applyFrame = 0;
  let restoreFrame = 0;

  const restore = () => {
    restoreFrame = 0;
    for (const [el, prev] of saved) {
      el.style.contain = prev.contain;
      el.style.contentVisibility = prev.contentVisibility;
      el.style.textShadow = prev.textShadow;
    }
    saved.clear();
  };

  const apply = () => {
    applyFrame = 0;
    let nudged = false;
    for (const el of queued) {
      if (!el.isConnected) continue;
      if (!saved.has(el)) {
        saved.set(el, {
          contain: el.style.contain,
          contentVisibility: el.style.contentVisibility,
          textShadow: el.style.textShadow,
        });
      }
      // Temporarily drop paint containment so WebKit rebuilds the layer and re-rasterizes stale selection tiles;
      // transparent text-shadow marks every text run (including inside sticky prompt layers) dirty without relayout
      el.style.contain = 'none';
      el.style.contentVisibility = 'visible';
      el.style.textShadow = '0 0 0 transparent';
      nudged = true;
    }
    queued.clear();
    if (nudged) {
      if (restoreFrame) cancelRaf(restoreFrame);
      restoreFrame = raf(restore);
    }
  };

  const nudgeRepaint = (elements: Iterable<HTMLElement>) => {
    if (!webkit) return;
    let added = false;
    for (const el of elements) {
      if (el.isConnected) {
        queued.add(el);
        added = true;
      }
    }
    if (added && !applyFrame) applyFrame = raf(apply);
  };

  const syncTracked = () => {
    if (!webkit) return;
    const next = selectedContained(doc.getSelection());
    const departed: HTMLElement[] = [];
    for (const el of tracked) {
      if (!next.has(el)) departed.push(el);
    }
    tracked = next;
    if (departed.length) nudgeRepaint(departed);
  };

  const clearSelection = (sel: Selection) => {
    if (webkit) {
      for (const el of selectedContained(sel)) tracked.add(el);
    }
    sel.removeAllRanges();
    syncTracked();
  };

  const onMouseDown = (e: MouseEvent) => {
    pending = undefined;
    // Ignore non-primary buttons (right-click copy), Shift-click selection extension, and double/triple-click selection
    if (e.button !== 0 || e.shiftKey || e.detail > 1) return;
    const sel = doc.getSelection();
    if (!sel || sel.isCollapsed || sel.rangeCount === 0 || selectionInInput(doc, sel)) return;
    const target = e.target;
    if (!isElement(target) || target.closest(PRESERVE) || onScrollbar(target, e)) return;
    if (isUnselectableChrome(target, e, getStyle)) {
      clearSelection(sel);
      return;
    }
    // Editable fields handle focus and selection natively
    if (target.closest(EDITABLE)) return;
    pending = {
      x: e.clientX,
      y: e.clientY,
      anchorNode: sel.anchorNode,
      anchorOffset: sel.anchorOffset,
      focusNode: sel.focusNode,
      focusOffset: sel.focusOffset,
    };
  };

  const onMouseUp = (e: MouseEvent) => {
    const snap = pending;
    pending = undefined;
    if (!snap || e.button !== 0 || e.shiftKey || e.detail > 1) return;
    if (Math.hypot(e.clientX - snap.x, e.clientY - snap.y) > CLICK_SLOP) return;
    const sel = doc.getSelection();
    if (sel && sameSelection(sel, snap)) clearSelection(sel);
  };

  const cancelPending = () => { pending = undefined; };

  doc.addEventListener('mousedown', onMouseDown, true);
  doc.addEventListener('mouseup', onMouseUp, true);
  doc.addEventListener('dragstart', cancelPending, true);
  doc.addEventListener('selectionchange', syncTracked);

  return () => {
    doc.removeEventListener('mousedown', onMouseDown, true);
    doc.removeEventListener('mouseup', onMouseUp, true);
    doc.removeEventListener('dragstart', cancelPending, true);
    doc.removeEventListener('selectionchange', syncTracked);
    if (applyFrame) cancelRaf(applyFrame);
    if (restoreFrame) cancelRaf(restoreFrame);
    queued.clear();
    restore();
    tracked.clear();
    pending = undefined;
  };
}
