import { useCallback, useContext, useEffect, useMemo, useRef, useState, type CSSProperties, type MouseEvent, type ReactElement } from 'react';
import { ContextMenu } from '@base-ui/react/context-menu';
import { Check, CodeXml, Copy, Eye, Image as ImageIcon, Maximize2, RotateCcw, ZoomIn, ZoomOut } from 'lucide-react';
import type { MermaidConfig } from '@streamdown/mermaid';
import { t, useLocale } from '../i18n';
import { useTheme, type Theme } from '../look';
import { IconButton } from '../ui/Button';
import { cn } from '../ui/cn';
import { Dialog } from '../ui/Dialog';
import { optionClass } from '../ui/DropdownMenu';
import { ShellLayerContext, popupClass } from '../ui/Overlay';
import { useCopied, useCopyAction } from '../ui/useCopied';
import { CodeBlock, CODE_CARD } from './CodeBlock';
import { useImageZoom } from './Lightbox';
import { mermaidDiagram } from './mermaidPlugin';

export interface SvgSize {
  width: number;
  height: number;
}

const VIEWBOX_RE = /\bviewBox\s*=\s*["']\s*[+-]?[\d.]+\s+[+-]?[\d.]+\s+([+-]?[\d.]+)\s+([+-]?[\d.]+)\s*["']/i;
const ATTR_W_RE = /<svg\b[^>]*\bwidth\s*=\s*["']([\d.]+)(?:px)?["']/i;
const ATTR_H_RE = /<svg\b[^>]*\bheight\s*=\s*["']([\d.]+)(?:px)?["']/i;
const MAX_CANVAS_PIXELS = 16_000_000;
const MAX_CANVAS_SIDE = 16_384;
// GPU-safe texture budget for the interactive lightbox bitmap: crisp at 1:1 and beyond without exceeding Metal / Skia limits.
const MAX_LIGHTBOX_CANVAS_PIXELS = 12_000_000;
const MAX_LIGHTBOX_CANVAS_SIDE = 8_192;
const MAX_LIGHTBOX_RASTER_SCALE = 4;
// Quiet time after the last zoom / pan write before the visible region is re-rasterized sharp.
const DETAIL_SETTLE_MS = 160;
const EXPORT_PAD = 24;
const EXPORT_DPR = 2;

export function computeLightboxCanvasSize(size: SvgSize, dpr = 2): { width: number; height: number; scale: number } {
  const safeDpr = Number.isFinite(dpr) && dpr > 0 ? dpr : 1;
  const desiredScale = Math.min(MAX_LIGHTBOX_RASTER_SCALE, Math.max(2, safeDpr * 1.5));
  const scale = Math.min(
    desiredScale,
    MAX_LIGHTBOX_CANVAS_SIDE / Math.max(1, size.width, size.height),
    Math.sqrt(MAX_LIGHTBOX_CANVAS_PIXELS / Math.max(1, size.width * size.height)),
  );
  return {
    width: Math.max(1, Math.round(size.width * scale)),
    height: Math.max(1, Math.round(size.height * scale)),
    scale,
  };
}

const contextMenuCollision = { side: 'flip', align: 'flip', fallbackAxisSide: 'none' } as const;
const positionerClass = 'z-50 w-max min-w-pop-sm';

// Intrinsic pixel dimensions from the SVG's viewBox (preferred: Mermaid writes `width="100%"` alongside a pixel viewBox),
// falling back to numeric width/height attributes.
export function parseSvgSize(svg: string): SvgSize | undefined {
  const vb = VIEWBOX_RE.exec(svg);
  if (vb) {
    const width = Number(vb[1]);
    const height = Number(vb[2]);
    if (Number.isFinite(width) && Number.isFinite(height) && width > 0 && height > 0) return { width, height };
  }
  const w = Number(ATTR_W_RE.exec(svg)?.[1]);
  const h = Number(ATTR_H_RE.exec(svg)?.[1]);
  if (Number.isFinite(w) && Number.isFinite(h) && w > 0 && h > 0) return { width: w, height: h };
  return undefined;
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

// Rewrites every declared SVG `id` and its internal `#id` / `url(#id)` / `href="#id"` references with a suffix so the
// inline card and the open Lightbox can mount the same rendered SVG concurrently without clashing markers or styles.
export function scopeSvgIds(svg: string, suffix: string): string {
  const ids = new Set<string>();
  for (const match of svg.matchAll(/\bid="([^"]+)"/g)) {
    if (match[1]) ids.add(match[1]);
  }
  if (!ids.size) return svg;
  // Longest first so a prefix id (e.g. `acp-mmd-1`) does not partially rewrite `acp-mmd-1_flowchart-pointEnd`.
  const pattern = [...ids].sort((a, b) => b.length - a.length).map(escapeRegExp).join('|');
  const idDecl = new RegExp(`\\bid="(${pattern})"`, 'g');
  const urlRef = new RegExp(`url\\((["']?)#(${pattern})\\1\\)`, 'g');
  const hrefRef = new RegExp(`((?:xlink:)?href=")#(${pattern})"`, 'g');
  const hashRef = new RegExp(`#(${pattern})(?![A-Za-z0-9_-])`, 'g');
  return svg
    .replace(idDecl, (_, id: string) => `id="${id}-${suffix}"`)
    .replace(urlRef, (_, q: string, id: string) => `url(${q}#${id}-${suffix}${q})`)
    .replace(hrefRef, (_, prefix: string, id: string) => `${prefix}#${id}-${suffix}"`)
    .replace(/<style\b[^>]*>[\s\S]*?<\/style>/gi, block => block.replace(hashRef, (_, id: string) => `#${id}-${suffix}`));
}

// Gives a Mermaid SVG explicit pixel dimensions and xmlns so it decodes at full crisp size inside an <img> for PNG export.
export function prepareSvgForExport(svg: string, size?: SvgSize): string {
  const dims = size ?? parseSvgSize(svg);
  return svg.replace(/<svg\b([^>]*)>/i, (_, attrs: string) => {
    let next = attrs.replace(/\s+(?:width|height)="[^"]*"/gi, '');
    if (!/\bxmlns=/.test(next)) next += ' xmlns="http://www.w3.org/2000/svg"';
    if (dims) next += ` width="${Math.ceil(dims.width)}" height="${Math.ceil(dims.height)}"`;
    return `<svg${next}>`;
  });
}

// Decodes the standalone SVG into an image canvas code can draw at any scale.
async function decodeDiagram(svg: string, size: SvgSize): Promise<HTMLImageElement> {
  const img = new Image();
  img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(prepareSvgForExport(svg, size))}`;
  await img.decode();
  return img;
}

// A zoomed diagram is a transformed layer, and WebKit composites a large transformed layer (inline SVG or canvas
// alike) at a capped contents scale, so zooming in stretches soft pixels. Once a view settles, this draws just the
// on-screen part of the vector SVG at device resolution into an opaque overlay canvas pinned 1:1 in screen space above
// the diagram (`detail`, absolutely positioned in `host`, never inside the transformed layer). It hides itself while
// `base` already shows at most `sharpPerUnit` device px per diagram unit. Callers hide it while a gesture moves.
function drawDiagramDetail({ detail, base, view, host, img, size, sharpPerUnit, background }: {
  detail: HTMLCanvasElement;
  base: HTMLElement;
  view: HTMLElement;
  host: HTMLElement;
  img: HTMLImageElement;
  size: SvgSize;
  sharpPerUnit: number;
  background: string;
}): void {
  const hide = () => { detail.style.display = 'none'; detail.width = 0; detail.height = 0; };
  const box = base.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  if (!box.width) return hide();
  const perUnit = box.width / size.width; // screen px per diagram unit
  if (perUnit * dpr <= sharpPerUnit * 1.05) return hide();
  const clip = view.getBoundingClientRect();
  // Snap to device pixels so the overlay bitmap maps 1:1 onto the screen.
  const snap = (v: number) => Math.round(v * dpr) / dpr;
  const left = snap(Math.max(box.left, clip.left));
  const top = snap(Math.max(box.top, clip.top));
  const right = snap(Math.min(box.right, clip.right));
  const bottom = snap(Math.min(box.bottom, clip.bottom));
  if (right <= left || bottom <= top) return hide();
  const w = Math.round((right - left) * dpr);
  const h = Math.round((bottom - top) * dpr);
  const origin = host.getBoundingClientRect();
  detail.width = w;
  detail.height = h;
  Object.assign(detail.style, {
    display: 'block',
    left: `${left - origin.left - host.clientLeft}px`,
    top: `${top - origin.top - host.clientTop}px`,
    width: `${w / dpr}px`,
    height: `${h / dpr}px`,
  });
  const ctx = detail.getContext('2d');
  if (!ctx) return hide();
  // Opaque fill so the soft layer underneath never shows through around the sharp glyphs.
  ctx.fillStyle = background;
  ctx.fillRect(0, 0, w, h);
  // Scale through the context transform and draw the whole SVG, letting the canvas bounds clip: WebKit rasterizes
  // an SVG at its intrinsic size for a source-rect drawImage, which would just stretch the same soft pixels.
  ctx.setTransform(perUnit * dpr, 0, 0, perUnit * dpr, (box.left - left) * dpr, (box.top - top) * dpr);
  // Draw twice for WebKit foreignObject reliability (docs/dev/webview.md)
  ctx.drawImage(img, 0, 0, size.width, size.height);
  ctx.drawImage(img, 0, 0, size.width, size.height);
}

// Settle-debounced redraw of a detail overlay: `schedule` hides the overlay at once (it cannot follow a moving view)
// and runs the latest `redraw` once writes stop for DETAIL_SETTLE_MS.
function useDetailLayer() {
  const detailRef = useRef<HTMLCanvasElement | null>(null);
  const redrawRef = useRef<() => void>(() => {});
  const timer = useRef<number | undefined>(undefined);
  const schedule = useCallback(() => {
    if (detailRef.current) detailRef.current.style.display = 'none';
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => redrawRef.current(), DETAIL_SETTLE_MS);
  }, []);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  return { detailRef, redrawRef, schedule };
}

// Rasterizes a rendered Mermaid SVG onto a padded HiDPI PNG so Copy image works cleanly via ClipboardItem.
async function renderMermaidToPng(svg: string, size: SvgSize | undefined, theme: Theme): Promise<Blob> {
  const dims = size ?? parseSvgSize(svg) ?? { width: 800, height: 600 };
  const standalone = prepareSvgForExport(svg, dims);
  const img = new Image();
  img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(standalone)}`;
  await img.decode();

  const logicalW = Math.ceil(dims.width) + EXPORT_PAD * 2;
  const logicalH = Math.ceil(dims.height) + EXPORT_PAD * 2;
  const scale = Math.min(
    EXPORT_DPR,
    MAX_CANVAS_SIDE / Math.max(logicalW, logicalH),
    Math.sqrt(MAX_CANVAS_PIXELS / Math.max(1, logicalW * logicalH)),
  );
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(logicalW * scale));
  canvas.height = Math.max(1, Math.round(logicalH * scale));
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('Canvas is unavailable');
  ctx.scale(scale, scale);
  ctx.fillStyle = theme === 'dark' ? '#16181d' : '#ffffff';
  ctx.fillRect(0, 0, logicalW, logicalH);
  // Draw twice for WebKit foreignObject reliability (docs/dev/webview.md)
  ctx.drawImage(img, EXPORT_PAD, EXPORT_PAD, dims.width, dims.height);
  ctx.drawImage(img, EXPORT_PAD, EXPORT_PAD, dims.width, dims.height);
  return await new Promise<Blob>((resolve, reject) => {
    canvas.toBlob(blob => (blob ? resolve(blob) : reject(new Error('Image conversion failed'))), 'image/png');
  });
}

async function copyMermaidImage(svg: string, size: SvgSize | undefined, theme: Theme): Promise<void> {
  if (!navigator.clipboard?.write || typeof ClipboardItem === 'undefined') throw new Error('Clipboard image write unavailable');
  // Hand ClipboardItem a pending PNG synchronously so WebKit's user gesture never expires while decoding.
  const png = renderMermaidToPng(svg, size, theme);
  await navigator.clipboard.write([new ClipboardItem({ 'image/png': png })]);
}

// Neutral dark/light Mermaid palettes matching Acpira's grey surfaces and hairline borders (instead of Mermaid's muddy
// default `#333` dark clusters).
// A concrete font list, never the bare `monospace` generic: the zoom overlay, Copy image and the lightbox draw the SVG
// as an image, whose `monospace` resolves to Courier while the page's resolves to its own fixed font, and CSS variables
// do not reach inside an image either. Named installed fonts resolve the same in both places.
const MERMAID_FONT_FALLBACK = '"SF Mono", Menlo, Monaco, Consolas, monospace';
function mermaidFont(): string {
  const token = typeof document === 'undefined' ? '' : getComputedStyle(document.documentElement).getPropertyValue('--font-mono').trim();
  return token && !token.includes('var(') ? token : MERMAID_FONT_FALLBACK;
}

function mermaidConfig(theme: Theme): MermaidConfig {
  const dark = theme === 'dark';
  const fontFamily = mermaidFont();
  return {
    startOnLoad: false,
    theme: 'base',
    fontFamily,
    themeVariables: dark ? {
      fontFamily,
      darkMode: true,
      background: 'transparent',
      primaryColor: '#1f2329',
      primaryTextColor: '#e4e7ec',
      primaryBorderColor: '#3e4552',
      secondaryColor: '#191d23',
      secondaryTextColor: '#d0d5dd',
      secondaryBorderColor: '#343a46',
      tertiaryColor: '#15181d',
      tertiaryTextColor: '#c3c9d4',
      tertiaryBorderColor: '#2e333e',
      lineColor: '#697487',
      textColor: '#dde2ea',
      mainBkg: '#1f2329',
      nodeBorder: '#3e4552',
      clusterBkg: 'rgba(255, 255, 255, 0.025)',
      clusterBorder: '#343a46',
      titleColor: '#e4e7ec',
      edgeLabelBackground: '#181b20',
      actorBkg: '#1f2329',
      actorBorder: '#3e4552',
      actorTextColor: '#e4e7ec',
      actorLineColor: '#485060',
      signalColor: '#8895aa',
      signalTextColor: '#dde2ea',
      sequenceNumberColor: '#111419',
      noteBkgColor: '#242933',
      noteTextColor: '#dde2ea',
      noteBorderColor: '#3e4552',
    } : {
      fontFamily,
      darkMode: false,
      background: 'transparent',
      primaryColor: '#f6f7f9',
      primaryTextColor: '#1f2329',
      primaryBorderColor: '#c9cfd8',
      secondaryColor: '#eff1f5',
      secondaryTextColor: '#2c313a',
      secondaryBorderColor: '#d2d7e0',
      tertiaryColor: '#fafafa',
      tertiaryTextColor: '#383e49',
      tertiaryBorderColor: '#d8dde5',
      lineColor: '#7a8394',
      textColor: '#1f2329',
      mainBkg: '#f6f7f9',
      nodeBorder: '#c9cfd8',
      clusterBkg: 'rgba(0, 0, 0, 0.02)',
      clusterBorder: '#d2d7e0',
      titleColor: '#1f2329',
      edgeLabelBackground: '#f6f7f9',
      actorBkg: '#f6f7f9',
      actorBorder: '#c9cfd8',
      actorTextColor: '#1f2329',
      actorLineColor: '#b8c0cc',
      signalColor: '#697282',
      signalTextColor: '#1f2329',
      sequenceNumberColor: '#ffffff',
      noteBkgColor: '#fffbeb',
      noteTextColor: '#2c313a',
      noteBorderColor: '#d8dde5',
    },
  };
}

// Right-click menu shared by the inline Mermaid card and the frameless Lightbox (matching ImageMenu / SharePreview).
function DiagramMenu({ svg, chart, size, theme, children }: {
  svg: string;
  chart: string;
  size?: SvgSize;
  theme: Theme;
  children: ReactElement<Record<string, unknown>>;
}) {
  const layer = useContext(ShellLayerContext);
  const writeImage = useCallback(() => copyMermaidImage(svg, size, theme), [svg, size, theme]);
  const writeCode = useCallback(() => navigator.clipboard.writeText(chart), [chart]);
  const imageCopy = useCopyAction(svg, writeImage);
  const codeCopy = useCopyAction(chart, writeCode);
  const status = imageCopy.state !== 'idle' ? imageCopy.state : codeCopy.state;
  return (
    <>
      <ContextMenu.Root>
        <ContextMenu.Trigger render={children} />
        <ContextMenu.Portal container={layer?.current ?? undefined}>
          <ContextMenu.Positioner className={positionerClass} collisionAvoidance={contextMenuCollision} collisionBoundary={layer?.current ?? undefined}>
            {/* Stop propagation so picking a menu item never hits the Lightbox's backdrop close handler */}
            <ContextMenu.Popup finalFocus={false} className={popupClass} onClick={e => e.stopPropagation()}>
              <ContextMenu.Item onClick={() => void imageCopy.copy()} className={optionClass}>
                <ImageIcon className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
                {t('image.copy')}
              </ContextMenu.Item>
              <ContextMenu.Item onClick={() => void codeCopy.copy()} className={optionClass}>
                <Copy className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
                {t('code.copyCode')}
              </ContextMenu.Item>
            </ContextMenu.Popup>
          </ContextMenu.Positioner>
        </ContextMenu.Portal>
      </ContextMenu.Root>
      <span className="sr-only" role="status">{status === 'idle' ? '' : t(status === 'copied' ? 'code.copied' : 'code.copyFailed')}</span>
    </>
  );
}

let mmdSeq = 0;

const SVG_FILL = '[&_svg]:block [&_svg]:size-full [&_svg]:max-w-none!';

// Mermaid diagram: rendered off-DOM via the plugin's shared instance; while streaming (or on bad syntax) the source shows
// as a code block. Supports both in-place zoom/pan (top-right toolbar + Ctrl/pinch zoom + drag pan) and a frameless
// full-shell Lightbox matching image previews (wheel/pinch zoom, drag pan, double-click 1:1 toggle, right-click copy).
export function MermaidBlock({ chart }: { chart: string }) {
  useLocale();
  const theme = useTheme();
  const [svg, setSvg] = useState<string>();
  const [failed, setFailed] = useState(false);
  const [showSource, setShowSource] = useState(false);
  const [expanded, setExpanded] = useState(false);

  useEffect(() => {
    let dead = false;
    setFailed(false);
    // Debounced: the chart text changes with every streamed chunk, and partial syntax usually fails to parse
    const timer = setTimeout(() => {
      const id = `acp-mmd-${++mmdSeq}`;
      mermaidDiagram
        .getMermaid(mermaidConfig(theme))
        .render(id, chart)
        .then(({ svg: rendered }) => { if (!dead) setSvg(rendered); })
        .catch(() => {
          // mermaid drops its error graphic into the document under the render id
          document.getElementById(id)?.remove();
          document.getElementById(`d${id}`)?.remove();
          if (!dead) { setSvg(undefined); setFailed(true); }
        });
    }, 200);
    return () => { dead = true; clearTimeout(timer); };
  }, [chart, theme]);

  const size = useMemo(() => (svg ? parseSvgSize(svg) : undefined), [svg]);
  const inlineDetail = useDetailLayer();
  const inlineZoom = useImageZoom(svg, { naturalSize: size, wheelMode: 'ctrl-only', onViewChange: inlineDetail.schedule });
  const cardRef = useRef<HTMLDivElement | null>(null);
  const surfaceElRef = useRef<HTMLElement | null>(null);
  const inlineImg = useRef<{ svg: string; img: HTMLImageElement } | null>(null);
  inlineDetail.redrawRef.current = () => {
    const detail = inlineDetail.detailRef.current;
    const base = inlineZoom.targetRef.current;
    const view = surfaceElRef.current;
    const host = cardRef.current;
    if (!detail || !base || !view || !host || !svg || !size) return;
    const draw = (img: HTMLImageElement) => drawDiagramDetail({
      detail, base, view, host, img, size,
      // At rest the inline SVG is vector-sharp at its layout size.
      sharpPerUnit: (base.offsetWidth / size.width) * (window.devicePixelRatio || 1),
      background: getComputedStyle(host).backgroundColor,
    });
    if (inlineImg.current?.svg === svg) return draw(inlineImg.current.img);
    const want = svg;
    void decodeDiagram(want, size).then(img => {
      inlineImg.current = { svg: want, img };
      if (svg === want) draw(img);
    }).catch(() => {});
  };
  const inlineSurfaceRef = useCallback((el: HTMLElement | null) => {
    surfaceElRef.current = el;
    inlineZoom.surfaceRef(el);
  }, [inlineZoom.surfaceRef]);
  // A width change reflows the card under an unchanged transform, so refresh the overlay.
  useEffect(() => {
    const el = surfaceElRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => inlineDetail.schedule());
    ro.observe(el);
    return () => ro.disconnect();
  }, [svg, showSource, inlineDetail.schedule]);
  const sourceCopy = useCopied(chart);

  if (failed || !svg) {
    return failed ? <CodeBlock code={chart} lang="mermaid" /> : <div className="min-h-10 animate-[acp-pulse_1.5s_ease_infinite] rounded-md bg-code" />;
  }

  if (showSource) {
    const copyLabel = sourceCopy.state === 'copied' ? t('code.copied') : sourceCopy.state === 'failed' ? t('code.copyFailed') : t('code.copyCode');
    return (
      <div className="group/mermaid relative min-w-0" data-mermaid-source>
        <div className="code-copy absolute z-3 top-gap-half right-gap-half flex items-center rounded-code-copy bg-bg-2 ring ring-conversation-line">
          <IconButton size="sm" className="rounded-code-copy" title={copyLabel} aria-label={copyLabel} onClick={sourceCopy.copy}>
            {sourceCopy.state === 'copied' ? <Check strokeWidth={1.5} /> : <Copy strokeWidth={1.5} />}
          </IconButton>
          <IconButton size="sm" className="rounded-code-copy" title={t('diagram.viewDiagram')} aria-label={t('diagram.viewDiagram')} onClick={() => setShowSource(false)}>
            <Eye strokeWidth={1.5} />
          </IconButton>
        </div>
        <span className="sr-only" role="status">{sourceCopy.state === 'idle' ? '' : copyLabel}</span>
        <CodeBlock code={chart} lang="mermaid" copy={false} />
      </div>
    );
  }

  const zoomed = inlineZoom.zoomed;
  const inlineBoxStyle: CSSProperties | undefined = size ? {
    width: `min(100%, ${size.width}px, calc(var(--mermaid-max-h) * ${size.width / size.height}))`,
    aspectRatio: `${size.width} / ${size.height}`,
  } : undefined;

  const onStageClick = () => {
    if (inlineZoom.consumeDrag() || inlineZoom.getScale() > 1.01) return;
    setExpanded(true);
  };

  const onStageDoubleClick = (e: MouseEvent) => {
    e.stopPropagation();
    if (inlineZoom.isTransformed()) {
      inlineZoom.reset();
    } else {
      setExpanded(true);
    }
  };

  return (
    <>
      <div
        ref={cardRef}
        data-mermaid
        className={cn('group/mermaid relative min-w-0 overflow-hidden bg-code', CODE_CARD)}
      >
        {/* Top-right floating controls (same surface & reveal cadence as OutputCopy on CodeBlock); Reset only appears
            at the leading edge while zoomed so the pill stays free of disabled buttons at rest and + never shifts */}
        <div
          onClick={e => e.stopPropagation()}
          onDoubleClick={e => e.stopPropagation()}
          className={cn(
            'code-copy absolute z-3 top-gap-half right-gap-half flex items-center rounded-code-copy bg-bg-2 ring ring-conversation-line transition-opacity duration-(--code-copy-duration) ease-out motion-reduce:transition-none',
            inlineZoom.transformed
              ? 'opacity-100'
              : 'opacity-0 group-hover/mermaid:opacity-100 group-focus-within/mermaid:opacity-100 [@media(hover:none)]:opacity-100',
          )}
        >
          {inlineZoom.transformed && (
            <IconButton size="sm" className="rounded-code-copy" title={t('diagram.reset')} aria-label={t('diagram.reset')} onClick={inlineZoom.reset}>
              <RotateCcw strokeWidth={1.5} />
            </IconButton>
          )}
          <IconButton size="sm" className="rounded-code-copy" title={t('diagram.zoomOut')} aria-label={t('diagram.zoomOut')} onClick={inlineZoom.zoomOut}>
            <ZoomOut strokeWidth={1.5} />
          </IconButton>
          <IconButton size="sm" className="rounded-code-copy" title={t('diagram.zoomIn')} aria-label={t('diagram.zoomIn')} onClick={inlineZoom.zoomIn}>
            <ZoomIn strokeWidth={1.5} />
          </IconButton>
          <IconButton size="sm" className="rounded-code-copy" title={t('diagram.viewSource')} aria-label={t('diagram.viewSource')} onClick={() => setShowSource(true)}>
            <CodeXml strokeWidth={1.5} />
          </IconButton>
          <IconButton size="sm" className="rounded-code-copy" title={t('diagram.expand')} aria-label={t('diagram.expand')} onClick={() => setExpanded(true)}>
            <Maximize2 strokeWidth={1.5} />
          </IconButton>
        </div>

        {/* Interactive canvas: plain wheel keeps scrolling the document; pinch/Ctrl+wheel or toolbar zooms in place,
            clicking at rest opens the frameless Lightbox, and right-click offers Copy image / Copy code */}
        <DiagramMenu svg={svg} chart={chart} size={size} theme={theme}>
          <div
            ref={inlineSurfaceRef}
            role="button"
            tabIndex={0}
            aria-label={t('diagram.expand')}
            onClick={onStageClick}
            onDoubleClick={onStageDoubleClick}
            onKeyDown={e => {
              if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                setExpanded(true);
              }
            }}
            onPointerDown={inlineZoom.onPointerDown}
            className={cn(
              'flex w-full items-center justify-center overflow-hidden p-pad outline-none select-none focus-visible:ring-1 focus-visible:ring-focus',
              zoomed ? 'min-h-(--mermaid-zoom-h) touch-none' : 'min-h-(--mermaid-min-h)',
              zoomed ? (inlineZoom.dragging ? 'cursor-grabbing' : 'cursor-grab') : 'cursor-zoom-in',
            )}
          >
            <div
              ref={inlineZoom.targetRef as React.RefObject<HTMLDivElement | null>}
              style={inlineBoxStyle}
              className={cn(
                'shrink-0',
                inlineZoom.transformed && 'will-change-transform',
                size ? SVG_FILL : '[&_svg]:h-auto [&_svg]:max-w-full',
              )}
              dangerouslySetInnerHTML={{ __html: svg }}
            />
          </div>
        </DiagramMenu>
        {/* Sharp overlay for a settled zoom; before the scale label in DOM order so the label stays on top */}
        <canvas ref={inlineDetail.detailRef} aria-hidden className="pointer-events-none absolute hidden" />

        {inlineZoom.transformed && (
          <div
            ref={inlineZoom.scaleLabelRef as React.RefObject<HTMLDivElement | null>}
            className="pointer-events-none absolute bottom-gap-half left-gap-half rounded-sm bg-bg-2/90 px-1.5 py-0.5 font-mono text-4 text-fg-3 ring ring-conversation-line tabular-nums"
          >
            {Math.round(inlineZoom.getScale() * 100)}%
          </div>
        )}
      </div>

      {expanded && <DiagramLightbox svg={svg} chart={chart} size={size} theme={theme} onClose={() => setExpanded(false)} />}
    </>
  );
}

// Frameless diagram Lightbox (same family as `Lightbox` and `SharePreviewDialog`): rasterizes the Mermaid SVG once
// onto a HiDPI `<canvas>` so wheel / pinch zooming runs as pure GPU texture compositing instead of re-rasterizing
// SVG `<foreignObject>` nodes (and avoids injecting a second `<style>` block into WebKit). Falls back to scoped inline
// SVG only when intrinsic dimensions are absent or bitmap decode fails.
function DiagramLightbox({ svg, chart, size, theme, onClose }: {
  svg: string;
  chart: string;
  size?: SvgSize;
  theme: Theme;
  onClose: () => void;
}) {
  useLocale();
  const [rasterFailed, setRasterFailed] = useState(false);
  const useFallbackSvg = !size || rasterFailed;
  const scopedSvg = useMemo(() => (useFallbackSvg ? scopeSvgIds(svg, 'lb') : undefined), [svg, useFallbackSvg]);
  // Sharp overlay for a settled zoom (drawDiagramDetail).
  const { detailRef, redrawRef: redrawDetailRef, schedule: scheduleDetail } = useDetailLayer();
  const zoom = useImageZoom(svg, { naturalSize: size, wheelMode: 'zoom', onViewChange: scheduleDetail });
  // Callback ref state (not useRef) because Dialog.Portal mounts its children after the initial effect runs.
  const [popup, setPopup] = useState<HTMLDivElement | null>(null);
  const [canvasEl, setCanvasEl] = useState<HTMLCanvasElement | null>(null);
  // Decoded standalone SVG plus the scale the base canvas was rasterized at, kept for detail redraws.
  const rasterRef = useRef<{ img: HTMLImageElement; scale: number } | null>(null);
  const [popupSize, setPopupSize] = useState<SvgSize>();

  useEffect(() => {
    setRasterFailed(false);
  }, [svg]);

  // Measure the popup viewport so wide or tall diagrams fit generously while small ones stay at 1:1 without upscaling.
  useEffect(() => {
    if (!popup) return;
    const measure = () => {
      const rect = popup.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) setPopupSize({ width: rect.width, height: rect.height });
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(popup);
    return () => ro.disconnect();
  }, [popup]);

  // Decode the standalone SVG once into the lightbox canvas so wheel / pinch zoom stays GPU-composited.
  useEffect(() => {
    if (!canvasEl || !size || rasterFailed) return;
    let dead = false;
    const raster = computeLightboxCanvasSize(size, window.devicePixelRatio || 1);
    const standalone = prepareSvgForExport(svg, size);
    const img = new Image();
    img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(standalone)}`;
    void img.decode().then(() => {
      if (dead) return;
      canvasEl.width = raster.width;
      canvasEl.height = raster.height;
      const ctx = canvasEl.getContext('2d');
      if (!ctx) {
        setRasterFailed(true);
        return;
      }
      ctx.setTransform(raster.scale, 0, 0, raster.scale, 0, 0);
      ctx.clearRect(0, 0, size.width, size.height);
      // Draw twice for WebKit foreignObject reliability (docs/dev/webview.md)
      ctx.drawImage(img, 0, 0, size.width, size.height);
      ctx.drawImage(img, 0, 0, size.width, size.height);
      rasterRef.current = { img, scale: raster.scale };
    }).catch(() => {
      if (!dead) setRasterFailed(true);
    });
    return () => {
      dead = true;
      rasterRef.current = null;
      canvasEl.width = 0;
      canvasEl.height = 0;
      const detail = detailRef.current;
      if (detail) {
        detail.width = 0;
        detail.height = 0;
        detail.style.display = 'none';
      }
    };
  }, [canvasEl, svg, size, rasterFailed]);

  // The base canvas is capped (MAX_LIGHTBOX_CANVAS_*), so past its raster scale a settled view gets the sharp overlay.
  redrawDetailRef.current = () => {
    const detail = detailRef.current;
    const raster = rasterRef.current;
    const stage = canvasEl?.parentElement;
    if (!detail || !canvasEl || !stage || !raster || !size || !popup) return;
    drawDiagramDetail({
      detail, base: canvasEl, view: popup, host: popup, img: raster.img, size,
      sharpPerUnit: raster.scale,
      background: getComputedStyle(stage).backgroundColor,
    });
  };

  // A resize moves the visible region, so refresh the detail layer too.
  useEffect(() => {
    if (popupSize) scheduleDetail();
  }, [popupSize, scheduleDetail]);

  const fitted = useMemo(() => {
    if (!size || !popupSize) return undefined;
    // Account for outer page padding + inner card padding so the fitted card never touches the shell edges.
    const availW = Math.max(160, popupSize.width - 72);
    const availH = Math.max(160, popupSize.height - 72);
    const ratio = size.width / size.height;
    const width = Math.min(availW, size.width, availH * ratio);
    const height = width / ratio;
    const fitRatio = width / size.width;
    return { width, height, fitRatio };
  }, [size, popupSize]);

  const actualTarget = fitted && fitted.fitRatio < 0.98 ? 1 / fitted.fitRatio : 1;
  const actualTargetRef = useRef(actualTarget);
  actualTargetRef.current = actualTarget;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key === '+' || e.key === '=') { e.preventDefault(); zoom.zoomIn(); }
      else if (e.key === '-') { e.preventDefault(); zoom.zoomOut(); }
      else if (e.key === '0') { e.preventDefault(); zoom.reset(); }
      else if (e.key === '1') {
        e.preventDefault();
        if (actualTargetRef.current > 1.02) zoom.zoomTo(actualTargetRef.current);
        else zoom.reset();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [zoom]);

  const popupRef = useCallback((el: HTMLDivElement | null) => {
    setPopup(el);
    zoom.surfaceRef(el);
  }, [zoom.surfaceRef]);

  const onDiagramDoubleClick = (e: MouseEvent) => {
    e.stopPropagation();
    if (zoom.isTransformed()) {
      zoom.reset();
    } else if (actualTarget > 1.05) {
      zoom.zoomTo(actualTarget, e.clientX, e.clientY);
    } else {
      zoom.zoomTo(2, e.clientX, e.clientY);
    }
  };

  return (
    <Dialog.Root open onOpenChange={open => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Popup
          ref={popupRef}
          aria-label={t('diagram.title')}
          onScrimClick={zoom.onBackdropClick(onClose)}
          className="absolute inset-0 z-40 flex touch-none items-center justify-center overflow-hidden p-page"
        >
          {/* Separate static backdrop so GPU-composited diagram zoom never forces backdrop-filter re-compositing */}
          <div aria-hidden className="pointer-events-none absolute inset-0 bg-scrim backdrop-blur-xs" />
          <DiagramMenu svg={svg} chart={chart} size={size} theme={theme}>
            <div
              ref={zoom.targetRef as React.RefObject<HTMLDivElement | null>}
              data-diagram-lightbox
              onClick={e => e.stopPropagation()}
              onDoubleClick={onDiagramDoubleClick}
              onPointerDown={zoom.onPointerDown}
              className={cn(
                'acp-share-stage relative z-1 flex shrink-0 items-center justify-center rounded-lg border border-line bg-bg-1 p-pad shadow-pop select-none will-change-transform',
                zoom.canPan && (zoom.dragging ? 'cursor-grabbing' : 'cursor-grab'),
              )}
            >
              {useFallbackSvg ? (
                <div
                  style={fitted ? { width: fitted.width, height: fitted.height } : undefined}
                  className={cn('shrink-0', fitted ? SVG_FILL : 'max-h-(--lightbox-max-h) max-w-full [&_svg]:h-auto [&_svg]:max-w-full')}
                  dangerouslySetInnerHTML={{ __html: scopedSvg ?? '' }}
                />
              ) : (
                <canvas
                  ref={setCanvasEl}
                  style={fitted ? { width: fitted.width, height: fitted.height } : undefined}
                  className={cn('block shrink-0', !fitted && 'max-h-(--lightbox-max-h) max-w-full')}
                />
              )}
            </div>
          </DiagramMenu>
          {!useFallbackSvg && <canvas ref={detailRef} aria-hidden className="pointer-events-none absolute z-2 hidden" />}
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
