// DOM cloning and SVG foreignObject rasterization for message and selection screenshots.
// Keeps zero runtime dependencies and respects the webview CSP (`img-src data:` allowed, `blob:` disallowed).

import { WIDGET_MESSAGE } from './widgetDoc';

const MAX_CANVAS_PIXELS = 16_000_000;
const TARGET_DPR = 2;
// How long one video frame or widget snapshot may take before the capture goes ahead without it
const MEDIA_TIMEOUT_MS = 5_000;
const BLANK_PIXEL = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7';

// Stand-in <img>s whose pixels are still being read (a video's first frame, a widget's snapshot); the capture waits on them
const pendingMedia = new WeakMap<HTMLImageElement, Promise<void>>();

// Cache inlined @font-face data URLs across captures so repeated shares do not re-fetch font files
const fontDataCache = new Map<string, Promise<string>>();

// Safe, readable file name derived from the session title (`Refactor auth` -> `Refactor-auth.png`)
export function shareFileName(sessionTitle: string): string {
  const cleaned = sessionTitle
    .trim()
    .replace(/[\\/:*?"<>|\u0000-\u001f]+/g, ' ')
    .replace(/\s+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 60);
  return `${cleaned || 'acpira-share'}.png`;
}

// Resolves which transcript turn a DOM node or selection belongs to, so a selection share picks that turn's model & effort
export function turnIndexOfNode(node: Node | null): number | undefined {
  const el = node instanceof Element ? node : node?.parentElement;
  if (!el) return undefined;
  const turnEl = el.closest('[data-turn]');
  if (turnEl) {
    const idx = Number(turnEl.getAttribute('data-turn'));
    if (Number.isInteger(idx) && idx >= 0) return idx;
  }
  const exchangeEl = el.closest('[data-exchange-start]');
  if (exchangeEl) {
    const start = Number(exchangeEl.getAttribute('data-exchange-start'));
    if (Number.isInteger(start) && start >= 0) return start;
  }
  return undefined;
}

// Clones an entire rendered message (`[data-part="agent-message"]` or `[data-part="user-message"]`) and strips interactive chrome
export function cloneMessageContent(source: HTMLElement): HTMLElement {
  const clone = cloneWithPaintedMedia(source);
  cleanShareSubtree(clone, true);
  return clone;
}

// Clones a user-selected DOM Range while preserving surrounding markdown / syntax-highlighting semantics
export function cloneSelectionContent(range: Range): HTMLElement {
  const fragment = range.cloneContents();
  const common = range.commonAncestorContainer;
  const commonEl = common instanceof Element ? common : common.parentElement;

  // Selection inside a fenced code block or command output: keep the syntax-highlighted <pre><code> card
  const pre = commonEl?.closest('pre');
  if (pre) {
    const code = commonEl?.closest('code');
    const preClone = pre.cloneNode(false) as HTMLElement;
    const codeClone = (code ? code.cloneNode(false) : document.createElement('code')) as HTMLElement;
    codeClone.appendChild(fragment);
    trimEmptyCodeEdges(codeClone);
    preClone.appendChild(codeClone);
    const wrap = document.createElement('div');
    wrap.className = 'flex min-w-0 flex-col';
    wrap.appendChild(preClone);
    cleanShareSubtree(wrap, false);
    return wrap;
  }

  // Rebuild ancestor typography wrappers up to the message boundary so list bullets, quotes, tables and `.acp-prose` rules apply
  let current: Node = fragment;
  const hasListItems = fragment.querySelector?.('li') !== null;
  const hasTableParts = fragment.querySelector?.('tr, td, th') !== null;

  for (
    let anc: Element | null = commonEl;
    anc && !anc.matches('[data-part="agent-message"], [data-part="user-message"], [data-exchange], [data-thread-body], [data-thread]');
    anc = anc.parentElement
  ) {
    const tag = anc.tagName.toLowerCase();
    // Single-line text selected inside one `<li>` reads cleaner as a paragraph than an orphaned bullet
    if ((tag === 'li' || tag === 'ul' || tag === 'ol') && !hasListItems) continue;
    if ((tag === 'td' || tag === 'th' || tag === 'tr' || tag === 'tbody' || tag === 'thead' || tag === 'table') && !hasTableParts) continue;
    if (
      tag === 'p' || tag === 'blockquote' || tag === 'ul' || tag === 'ol' || tag === 'li'
      || tag === 'table' || tag === 'thead' || tag === 'tbody' || tag === 'tr'
      || /^h[1-6]$/.test(tag)
      || anc.classList.contains('acp-prose')
      || anc.classList.contains('acp-thought')
    ) {
      const shell = anc.cloneNode(false) as Element;
      shell.removeAttribute('id');
      shell.appendChild(current);
      current = shell;
    }
  }

  const root = document.createElement('div');
  root.className = 'flex min-w-0 flex-col gap-gap text-1 text-fg-1';
  if (current instanceof HTMLElement && current.classList.contains('acp-prose')) {
    root.appendChild(current);
  } else {
    const prose = document.createElement('div');
    prose.className = 'acp-prose flex min-w-0 flex-col gap-gap';
    prose.appendChild(current);
    root.appendChild(prose);
  }

  pruneEmptyEdges(root);
  cleanShareSubtree(root, false);
  return root;
}

// Deep-clones an element and turns live media into static <img>s: canvases and playing videos at their current frame,
// a video without a decoded frame (never scrolled into view, metadata only) at its first frame read in the background,
// and a widget frame at the snapshot its page draws of itself (a sandboxed frame cannot be read from here)
function cloneWithPaintedMedia(source: HTMLElement): HTMLElement {
  const clone = source.cloneNode(true) as HTMLElement;
  const srcCanvases = source.querySelectorAll('canvas');
  const dstCanvases = clone.querySelectorAll('canvas');
  srcCanvases.forEach((canvas, i) => {
    const dst = dstCanvases[i];
    if (!dst) return;
    try {
      const img = document.createElement('img');
      img.src = canvas.toDataURL('image/png');
      img.className = dst.className;
      img.style.cssText = dst.style.cssText;
      if (canvas.clientWidth) img.style.width = `${canvas.clientWidth}px`;
      if (canvas.clientHeight) img.style.height = `${canvas.clientHeight}px`;
      dst.replaceWith(img);
    } catch { /* Tainted or zero-sized canvas stays as-is */ }
  });

  // An animated GIF plays as a video over an empty stand-in: the still keeps the GIF itself, which an SVG paints at its first frame
  clone.querySelectorAll<HTMLImageElement>('img[data-share-src]').forEach(img => { img.src = img.dataset.shareSrc ?? img.src; });
  clone.querySelectorAll('[data-share-skip]').forEach(el => el.remove());
  const srcVideos = source.querySelectorAll<HTMLVideoElement>('video:not([data-share-skip])');
  const dstVideos = clone.querySelectorAll('video');
  srcVideos.forEach((video, i) => {
    const dst = dstVideos[i];
    if (!dst) return;
    const img = standIn(dst.className, dst.style.cssText);
    dst.replaceWith(img);
    const now = video.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA ? frameOf(video) : undefined;
    const url = video.currentSrc || video.dataset.shareSrc;
    if (now) img.src = now;
    else if (url) fillLater(img, firstFrame(url));
  });

  // A widget that has not loaded (never scrolled near) or never answers is left out rather than kept as an empty box
  const srcWidgets = source.querySelectorAll<HTMLElement>('[data-share-widget]');
  const dstWidgets = clone.querySelectorAll<HTMLElement>('[data-share-widget]');
  srcWidgets.forEach((widget, i) => {
    const dst = dstWidgets[i];
    if (!dst) return;
    const frame = widget.querySelector('iframe');
    const win = frame?.contentWindow;
    const dstFrame = dst.querySelector('iframe');
    if (!frame || !win || !dstFrame) { dst.remove(); return; }
    // Sized like the frame until the snapshot lands, then by the snapshot itself (the card may be wider than the transcript)
    const img = standIn('block w-full max-w-full', `height:${frame.offsetHeight}px`);
    dstFrame.replaceWith(img);
    // Handled: the capture clones the card again and must not take the swapped widget for one that never loaded
    dst.removeAttribute('data-share-widget');
    fillLater(img, widgetSnapshot(win), { ownHeight: true, onMiss: () => dst.remove() });
  });
  return clone;
}

function standIn(className: string, style: string): HTMLImageElement {
  const img = document.createElement('img');
  img.src = BLANK_PIXEL;
  img.className = className;
  img.style.cssText = style;
  return img;
}

function fillLater(img: HTMLImageElement, pixels: Promise<string | undefined>, { ownHeight = false, onMiss }: { ownHeight?: boolean; onMiss?: () => void } = {}) {
  const timeout = new Promise<undefined>(resolve => setTimeout(resolve, MEDIA_TIMEOUT_MS));
  pendingMedia.set(img, Promise.race([pixels, timeout]).catch(() => undefined).then(async src => {
    if (!src) { onMiss?.(); return; }
    img.src = src;
    // Until the new pixels decode, WebKit lays the image out at the old 1x1 stand-in's square ratio
    await img.decode().catch(() => {});
    if (ownHeight) img.style.height = 'auto';
  }));
}

// The frame on screen; undefined when the canvas comes out tainted (a cross-origin source) or empty
function frameOf(video: HTMLVideoElement): string | undefined {
  if (!video.videoWidth || !video.videoHeight) return undefined;
  try {
    const canvas = document.createElement('canvas');
    canvas.width = video.videoWidth;
    canvas.height = video.videoHeight;
    canvas.getContext('2d')?.drawImage(video, 0, 0);
    return canvas.toDataURL('image/png');
  } catch { return undefined; }
}

// A recording's first frame from a detached player: straight from its URL first, then from fetched bytes where the
// resource origin taints the canvas (VS Code serves webview resources from another origin)
async function firstFrame(url: string): Promise<string | undefined> {
  const direct = await frameFrom(url).catch(() => undefined);
  if (direct) return direct;
  const res = await fetch(url);
  if (!res.ok) return undefined;
  const local = URL.createObjectURL(await res.blob());
  try { return await frameFrom(local); } finally { URL.revokeObjectURL(local); }
}

async function frameFrom(url: string): Promise<string | undefined> {
  const video = document.createElement('video');
  video.muted = true;
  video.playsInline = true;
  video.preload = 'auto';
  const settled = (event: string) => new Promise<void>((resolve, reject) => {
    video.addEventListener(event, () => resolve(), { once: true });
    video.addEventListener('error', () => reject(new Error('video load failed')), { once: true });
  });
  try {
    const loaded = settled('loadeddata');
    video.src = url;
    await loaded;
    // WebKit paints a paused frame only after a seek
    const seeked = settled('seeked');
    video.currentTime = 0.001;
    await seeked;
    return frameOf(video);
  } finally {
    video.removeAttribute('src');
    video.load();
  }
}

// The widget page draws itself (widgetDoc.ts BOOTSTRAP) and posts the PNG back; a page that never answers times out
function widgetSnapshot(win: Window): Promise<string | undefined> {
  return new Promise(resolve => {
    const id = Math.random().toString(36).slice(2);
    // Asked again until it answers: a frame still loading has no listener yet and drops the message
    const ask = () => win.postMessage({ [WIDGET_MESSAGE]: 'snapshot', id, dpr: TARGET_DPR }, '*');
    const retry = setInterval(ask, 250);
    const done = (png: string | undefined) => {
      clearInterval(retry);
      window.removeEventListener('message', onMessage);
      resolve(png);
    };
    const onMessage = (e: MessageEvent) => {
      const data = e.data as { [WIDGET_MESSAGE]?: string; id?: unknown; png?: unknown } | null;
      if (e.source !== win || data?.[WIDGET_MESSAGE] !== 'snapshot' || data.id !== id) return;
      done(typeof data.png === 'string' && data.png.startsWith('data:image/png') ? data.png : undefined);
    };
    window.addEventListener('message', onMessage);
    setTimeout(() => done(undefined), MEDIA_TIMEOUT_MS);
    ask();
  });
}

// Removes a node and every wrapper it leaves empty (no text, no media): an emptied wrapper still takes a flex gap,
// which showed as unexplained space above the reply
function removeWithEmptyWrappers(node: Element, root: HTMLElement) {
  let parent = node.parentElement;
  node.remove();
  while (parent && parent !== root && !parent.textContent?.trim() && !parent.querySelector('img, video, canvas, svg, iframe')) {
    const next = parent.parentElement;
    parent.remove();
    parent = next;
  }
}

function cleanShareSubtree(root: HTMLElement, stripRootPad: boolean) {
  root.querySelectorAll('[data-turn-actions], .code-copy, .sr-only').forEach(n => n.remove());
  // A whole reply is the agent's answer: the reader's own steered prompts and the reasoning stay out, folded or not.
  // A process fold's head goes too, unless the reader opened it and tool rows are still left under it
  if (stripRootPad) {
    root.querySelectorAll('.steered-message, .thought-row').forEach(n => removeWithEmptyWrappers(n, root));
    // Each fold's head is its first trigger, taken before any row is removed
    const folds = [...root.querySelectorAll<HTMLElement>('[data-process-fold]')].map(fold => ({ fold, head: fold.querySelector('button[aria-expanded]') }));
    // Collapsed rows (tool calls the reader never opened) go with their head when the reply has prose to show
    if (root.querySelector('.acp-prose')) {
      root.querySelectorAll<HTMLElement>('button[aria-expanded="false"]').forEach(trigger => {
        const row = trigger.parentElement;
        if (row && !row.querySelector('.acp-prose')) removeWithEmptyWrappers(row, root);
      });
    }
    for (const { fold, head } of folds) {
      const rows = fold.querySelector('.process-content');
      if (fold.hasAttribute('data-open') && rows?.textContent?.trim()) continue;
      if (head?.parentElement) removeWithEmptyWrappers(head.parentElement, root);
    }
  }
  if (stripRootPad) root.classList.remove('px-pad', '-mt-msg-join');
  root.classList.remove('fade-in', 'sticky');
  root.classList.add('pointer-events-none', 'select-none');

  // Expand scroll-capped blocks and wrap long code lines so screenshots show full text without scrollbars or fade masks
  root.querySelectorAll<HTMLElement>('[data-sticky-prompt]').forEach(el => {
    el.classList.remove('sticky', 'top-0', 'pointer-events-none');
  });
  root.querySelectorAll<HTMLElement>('[data-more-above], [data-more-below], .scroll-fade').forEach(el => {
    el.removeAttribute('data-more-above');
    el.removeAttribute('data-more-below');
    el.style.maxHeight = 'none';
    el.style.overflow = 'visible';
  });
  root.querySelectorAll<HTMLElement>('pre').forEach(pre => {
    pre.style.whiteSpace = 'pre-wrap';
    pre.style.overflowWrap = 'anywhere';
    pre.style.overflow = 'hidden';
  });
  root.querySelectorAll<HTMLElement>('.fade-in').forEach(el => el.classList.remove('fade-in'));
}

function trimEmptyCodeEdges(codeEl: HTMLElement) {
  while (codeEl.firstChild && codeEl.firstChild.textContent === '\n') codeEl.firstChild.remove();
  while (codeEl.lastChild && codeEl.lastChild.textContent === '') codeEl.lastChild.remove();
}

function pruneEmptyEdges(root: HTMLElement) {
  const isEmpty = (el: Element) => !el.textContent?.trim() && !el.querySelector('img, svg, canvas, hr, input');
  for (const leaf of [...root.querySelectorAll('p, li, h1, h2, h3, h4, h5, h6, blockquote')]) {
    if (isEmpty(leaf)) leaf.remove();
  }
}

// Rasterizes a live ShareCard element into a high-DPI PNG Blob
export async function renderShareCardToPng(cardEl: HTMLElement): Promise<Blob> {
  await Promise.all([...cardEl.querySelectorAll('img')].map(img => pendingMedia.get(img)));
  const rect = cardEl.getBoundingClientRect();
  const width = Math.max(1, cardEl.offsetWidth || Math.ceil(rect.width));
  const height = Math.max(1, cardEl.offsetHeight || Math.ceil(rect.height));
  const dpr = Math.max(1, Math.min(TARGET_DPR, Math.sqrt(MAX_CANVAS_PIXELS / (width * height))));

  const clonedCard = cloneWithPaintedMedia(cardEl);
  await inlineImages(clonedCard);
  // Kept alive until the draw below: WebKit only starts decoding an SVG's nested images on its first draw and paints
  // them blank there, unless the same data URLs are already decoded
  const decoded = await predecode(clonedCard);

  const cssText = await collectStylesheets();
  const shellEl = cardEl.closest<HTMLElement>('.acp-shell');
  const shellVars = collectCustomProperties(shellEl);

  const wrapper = document.createElement('div');
  wrapper.setAttribute('xmlns', 'http://www.w3.org/1999/xhtml');
  wrapper.className = document.body.className;
  wrapper.style.cssText = `margin:0;padding:0;width:${width}px;height:${height}px;overflow:hidden;background:transparent;`;

  const shellWrap = document.createElement('div');
  shellWrap.className = shellEl ? shellEl.className : 'acp-shell';
  if (shellEl) {
    for (const attr of shellEl.getAttributeNames()) {
      if (attr.startsWith('data-')) shellWrap.setAttribute(attr, shellEl.getAttribute(attr) ?? '');
    }
  }
  shellWrap.style.cssText = `${shellVars};width:${width}px;height:${height}px;overflow:visible;background:transparent;`;

  const styleEl = document.createElement('style');
  styleEl.textContent = cssText;
  wrapper.appendChild(styleEl);
  shellWrap.appendChild(clonedCard);
  wrapper.appendChild(shellWrap);

  const serialized = new XMLSerializer().serializeToString(wrapper);
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><foreignObject x="0" y="0" width="${width}" height="${height}">${serialized}</foreignObject></svg>`;
  const svgUrl = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;

  const img = new Image();
  img.decoding = 'sync';
  img.src = svgUrl;
  await img.decode();

  const canvas = document.createElement('canvas');
  canvas.width = Math.round(width * dpr);
  canvas.height = Math.round(height * dpr);
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('Canvas 2D context is unavailable');
  ctx.scale(dpr, dpr);
  ctx.drawImage(img, 0, 0, width, height);
  // A second draw after a task picks up any nested image WebKit was still decoding during the first
  if (decoded.length) {
    await new Promise(resolve => setTimeout(resolve, 32));
    ctx.clearRect(0, 0, width, height);
    ctx.drawImage(img, 0, 0, width, height);
  }
  decoded.length = 0;

  return new Promise<Blob>((resolve, reject) => {
    canvas.toBlob(blob => (blob ? resolve(blob) : reject(new Error('PNG encoding failed'))), 'image/png');
  });
}

async function predecode(root: HTMLElement): Promise<HTMLImageElement[]> {
  const urls = new Set([...root.querySelectorAll('img')].map(img => img.getAttribute('src') ?? '').filter(src => src.startsWith('data:')));
  return Promise.all([...urls].map(async src => {
    const img = new Image();
    img.src = src;
    await img.decode().catch(() => {});
    return img;
  }));
}

async function inlineImages(root: HTMLElement): Promise<void> {
  const imgs = [...root.querySelectorAll('img')];
  await Promise.all(imgs.map(async img => {
    const src = img.getAttribute('src') || '';
    img.removeAttribute('srcset');
    if (!src || src.startsWith('data:')) return;
    try {
      const res = await fetch(src);
      if (!res.ok) return;
      const blob = await res.blob();
      img.src = await blobToDataUrl(blob);
    } catch { /* Keep original URL if unreachable */ }
  }));
}

async function collectStylesheets(): Promise<string> {
  const chunks: string[] = [];
  for (const sheet of [...document.styleSheets]) {
    let raw = '';
    try {
      raw = [...sheet.cssRules].map(rule => rule.cssText).join('\n');
    } catch {
      if (sheet.href) {
        try {
          const res = await fetch(sheet.href);
          if (res.ok) raw = await res.text();
        } catch { /* Ignore unreachable external sheet */ }
      }
    }
    if (raw) chunks.push(await inlineFontFaces(raw, sheet.href ?? location.href));
  }
  return chunks.join('\n');
}

async function inlineFontFaces(css: string, baseHref: string): Promise<string> {
  if (!css.includes('@font-face')) return css;
  const blocks = [...css.matchAll(/@font-face\s*\{[^}]*\}/g)];
  if (!blocks.length) return css;
  const loadedFamilies = typeof document !== 'undefined' && document.fonts
    ? new Set([...document.fonts].filter(f => f.status === 'loaded').map(f => f.family.replace(/^['"]|['"]$/g, '')))
    : undefined;
  let result = css;
  for (const [block] of blocks) {
    if (loadedFamilies) {
      const fam = /font-family\s*:\s*(['"]?)([^;'"]+)\1/i.exec(block)?.[2]?.trim();
      if (fam && !loadedFamilies.has(fam)) continue;
    }
    let replaced = block;
    const urls = [...block.matchAll(/url\((['"]?)([^'")]+)\1\)/g)];
    for (const [full, , rawUrl] of urls) {
      if (!rawUrl || rawUrl.startsWith('data:')) continue;
      let resolved: string;
      try { resolved = new URL(rawUrl, baseHref).href; } catch { continue; }
      const dataUrl = await fetchFontDataUrl(resolved);
      if (dataUrl) replaced = replaced.replace(full, `url("${dataUrl}")`);
    }
    if (replaced !== block) result = result.replace(block, replaced);
  }
  return result;
}

function fetchFontDataUrl(url: string): Promise<string> {
  let pending = fontDataCache.get(url);
  if (!pending) {
    pending = fetch(url)
      .then(res => (res.ok ? res.blob() : Promise.reject(new Error('font fetch failed'))))
      .then(blobToDataUrl)
      .catch(() => '');
    fontDataCache.set(url, pending);
  }
  return pending;
}

function collectCustomProperties(shellEl: HTMLElement | null): string {
  const props = new Map<string, string>();
  const readInline = (style: CSSStyleDeclaration | undefined) => {
    if (!style) return;
    for (let i = 0; i < style.length; i++) {
      const name = style[i];
      if (name?.startsWith('--')) props.set(name, style.getPropertyValue(name));
    }
  };
  readInline(document.documentElement.style);
  readInline(document.body.style);
  readInline(shellEl?.style);

  // Preserve host-injected theme variables (--vscode-*) and resolved core surface/text variables
  const computed = getComputedStyle(shellEl ?? document.documentElement);
  for (const key of [
    '--fg-1', '--fg-2', '--fg-3', '--fg-strong', '--action-fg',
    '--bg-0', '--bg-1', '--bg-2', '--bg-code',
    '--line', '--line-strong', '--conversation-line', '--conversation-line-focus',
    '--chip', '--active', '--hover', '--accent', '--ok', '--warn', '--danger', '--fast', '--ultra',
    '--font', '--font-mono', '--ui-font-size', '--code-font-size',
  ]) {
    const val = computed.getPropertyValue(key).trim();
    if (val && !props.has(key)) props.set(key, val);
  }
  return [...props.entries()].map(([k, v]) => `${k}:${v}`).join(';');
}

export function blobToDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(reader.error ?? new Error('Failed to encode blob'));
    reader.readAsDataURL(blob);
  });
}
