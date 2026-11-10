// DOM cloning and SVG foreignObject rasterization for message and selection screenshots.
// Keeps zero runtime dependencies and respects the webview CSP (`img-src data:` allowed, `blob:` disallowed).

const MAX_CANVAS_PIXELS = 16_000_000;
const TARGET_DPR = 2;

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

// Deep-clones an element while snapshotting live <canvas> and <video> frames into static <img> elements
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

  const srcVideos = source.querySelectorAll('video');
  const dstVideos = clone.querySelectorAll('video');
  srcVideos.forEach((video, i) => {
    const dst = dstVideos[i];
    if (!dst || !video.videoWidth || !video.videoHeight) return;
    try {
      const c = document.createElement('canvas');
      c.width = video.videoWidth;
      c.height = video.videoHeight;
      c.getContext('2d')?.drawImage(video, 0, 0);
      const img = document.createElement('img');
      img.src = c.toDataURL('image/png');
      img.className = dst.className;
      img.style.cssText = dst.style.cssText;
      dst.replaceWith(img);
    } catch { /* Cross-origin or unloaded video frame */ }
  });
  return clone;
}

function cleanShareSubtree(root: HTMLElement, stripRootPad: boolean) {
  root.querySelectorAll('[data-turn-actions], .code-copy, .sr-only').forEach(n => n.remove());
  // When capturing a full message that has prose content, drop collapsed process fold headers ("Completed 3s >") unless
  // the user explicitly expanded them or a steered message sits inside
  if (stripRootPad && root.querySelector('.acp-prose')) {
    root.querySelectorAll<HTMLElement>('button[aria-expanded="false"]').forEach(trigger => {
      const foldRoot = trigger.parentElement;
      if (foldRoot && !foldRoot.querySelector('.steered-message, .acp-prose')) {
        foldRoot.remove();
      }
    });
  }
  if (stripRootPad) {
    root.classList.remove('px-pad', '-mt-msg-join');
    root.querySelectorAll('.steered-message').forEach(el => el.classList.remove('-mx-pad'));
  }
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
  const rect = cardEl.getBoundingClientRect();
  const width = Math.max(1, cardEl.offsetWidth || Math.ceil(rect.width));
  const height = Math.max(1, cardEl.offsetHeight || Math.ceil(rect.height));
  const dpr = Math.max(1, Math.min(TARGET_DPR, Math.sqrt(MAX_CANVAS_PIXELS / (width * height))));

  const clonedCard = cloneWithPaintedMedia(cardEl);
  await inlineImages(clonedCard);

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

  return new Promise<Blob>((resolve, reject) => {
    canvas.toBlob(blob => (blob ? resolve(blob) : reject(new Error('PNG encoding failed'))), 'image/png');
  });
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
