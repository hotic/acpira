import { memo, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { FileCode } from 'lucide-react';
import type { ToolCallBlock } from '@acpira/shared/transcript';
import { toolWidget } from '@acpira/shared/imageTools';
import { useAppearance } from '../appearance';
import { useTheme } from '../look';
import { t, useLocale } from '../i18n';
import { cn } from '../ui/cn';
import { BlobUrlContext, OpenToolFileContext, WidgetPromptContext, parseFileLink } from './fileLinks';
import { useShownBlob } from './stagedBlob';
import { WIDGET_MESSAGE, WIDGET_PROMPT_MAX, buildWidgetDoc, pageNonce, readWidgetTheme, type WidgetPageMsg } from './widgetDoc';

// An HTML widget an agent showed with Acpira's `show_widget`: its page runs in a sandboxed srcdoc frame (see
// `widgetDoc.ts` for the CSP and the head it gets), sized to the height the frame reports and following the editor theme
// by message, so a theme switch keeps the widget's state. The frame mounts once it nears the viewport and stays mounted.

// Tailwind's browser build is its own chunk, fetched by the first widget
let tailwind: Promise<string> | undefined;
const loadTailwind = () => tailwind ??= import('./widgetRuntime').then(m => m.tailwind);
let nonce: string | undefined;

// The frames' windows: the page's own message handlers drop anything they send (a widget could otherwise forge a host message)
const frames = new WeakSet<object>();
export const isWidgetFrame = (source: MessageEventSource | null): boolean => !!source && frames.has(source);

// The last height each widget reported, by blob: a remount (the process fold closing, a session switch) takes its place at once
const heights = new Map<string, number>();

// The shortest gap between two prompts of one widget: a double click or a handler bound twice sends once
const PROMPT_GAP_MS = 1000;

// The blob is written while the row completes, so the first read can miss it
async function fetchText(url: string): Promise<string> {
  for (let attempt = 0; ; attempt++) {
    const res = await fetch(url).catch(() => undefined);
    if (res?.ok) return res.text();
    if (attempt >= 4) throw new Error(`widget ${url}: ${res?.status ?? 'unreachable'}`);
    await new Promise(r => setTimeout(r, 250 * (attempt + 1)));
  }
}

// The widget of a completed show_widget row; nothing while it runs or when it failed (the row says so). One kept as a
// path (a session on an SSH host) is read by the host first, its placeholder holding the place meanwhile
export const ShownWidget = memo(function ShownWidget({ block }: { block: ToolCallBlock }) {
  // memo keeps a row from rendering again; the locale context still reaches it after a language change
  useLocale();
  const widget = toolWidget(block);
  const blob = useShownBlob(widget?.blob, widget?.uri);
  if (!widget) return null;
  if (blob === undefined) return <div aria-busy data-share-widget className="h-widget-placeholder w-full animate-pulse rounded-md bg-chip" />;
  // The host could not read the file (gone, or the host is unreachable): said in place of the frame
  if (blob === null) return <div role="status" className="text-3 text-fg-3">{t('panel.error.media')}</div>;
  return <AgentWidget blob={blob} uri={widget.uri} title={block.target} />;
});

function AgentWidget({ blob, uri, title }: { blob: string; uri?: string; title?: string }) {
  const blobUrl = useContext(BlobUrlContext);
  const openFile = useContext(OpenToolFileContext);
  const sendPrompt = useContext(WidgetPromptContext);
  const appearance = useAppearance();
  const dark = useTheme() === 'dark';
  const root = useRef<HTMLDivElement>(null);
  const frame = useRef<HTMLIFrameElement | null>(null);
  const [near, setNear] = useState(false);
  const [doc, setDoc] = useState<string>();
  const [failed, setFailed] = useState(false);
  const [height, setHeight] = useState(() => heights.get(blob));
  // The frame fades in on its first report after mounting, a remembered height only holds its place until then
  const [ready, setReady] = useState(false);
  const darkNow = useRef(dark);
  darkNow.current = dark;
  const file = uri ? parseFileLink(uri) : undefined;
  const name = file?.path.split(/[\\/]/).pop() ?? title ?? '';

  useEffect(() => {
    const element = root.current;
    if (!element) return;
    const observer = new IntersectionObserver(entries => {
      if (entries.some(e => e.isIntersecting)) { setNear(true); observer.disconnect(); }
    }, { rootMargin: '50% 0px' });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!near || !blobUrl) return;
    let alive = true;
    Promise.all([fetchText(blobUrl(blob)), loadTailwind()]).then(([html, tw]) => {
      if (!alive || !root.current) return;
      setDoc(buildWidgetDoc(html, { nonce: nonce ??= pageNonce(), tailwind: tw, theme: readWidgetTheme(root.current, darkNow.current) }));
    }, () => { if (alive) setFailed(true); });
    return () => { alive = false; };
  }, [near, blobUrl, blob]);

  // Theme and appearance changes go to the running page; the document it loaded already carries the theme of its build
  useEffect(() => {
    const win = frame.current?.contentWindow;
    if (!win || !doc || !root.current) return;
    const msg: WidgetPageMsg = { [WIDGET_MESSAGE]: 'theme', ...readWidgetTheme(root.current, dark) };
    win.postMessage(msg, '*');
  }, [dark, appearance, doc]);

  useEffect(() => {
    let lastPrompt = 0;
    const onMessage = (e: MessageEvent) => {
      if (!frame.current || e.source !== frame.current.contentWindow) return;
      const data = e.data as { [WIDGET_MESSAGE]?: string; height?: unknown; text?: unknown } | null;
      if (data?.[WIDGET_MESSAGE] === 'prompt') {
        // The frame's own check (user activation) runs in the widget's code, which could post this itself: the page also
        // wants the frame focused, which a click or key inside it does and a script alone cannot
        const text = typeof data.text === 'string' ? data.text.trim() : '';
        const now = Date.now();
        if (!sendPrompt || !text || text.length > WIDGET_PROMPT_MAX || document.activeElement !== frame.current || now - lastPrompt < PROMPT_GAP_MS) return;
        lastPrompt = now;
        sendPrompt(text);
        return;
      }
      if (data?.[WIDGET_MESSAGE] !== 'height' || typeof data.height !== 'number' || !(data.height > 0)) return;
      heights.set(blob, data.height);
      setHeight(data.height);
      setReady(true);
    };
    window.addEventListener('message', onMessage);
    return () => window.removeEventListener('message', onMessage);
  }, [blob, sendPrompt]);

  const attach = useCallback((el: HTMLIFrameElement | null) => {
    frame.current = el;
    if (el?.contentWindow) frames.add(el.contentWindow);
  }, []);

  if (failed) return (
    <button type="button" title={uri ?? name} onClick={file && openFile ? () => openFile(file.path, file.line) : undefined} disabled={!(file && openFile)}
      className="inline-flex h-ctl-sm max-w-full min-w-0 items-center gap-1 self-start rounded-sm bg-chip px-2 text-3 font-medium text-fg-2 disabled:cursor-default enabled:cursor-pointer enabled:hover:bg-chip-hover [&_svg]:size-icon [&_svg]:shrink-0 [&_svg]:text-fg-3">
      <FileCode strokeWidth={1.5} />
      <span className="truncate">{t('widget.failed')}{name && ` ｜ ${name}`}</span>
    </button>
  );
  return (
    // `data-share-widget`: a reply screenshot swaps the frame for the snapshot the page draws of itself, or leaves the
    // widget out when none comes back (shareCapture.ts)
    <div ref={root} data-share-widget className="relative w-full min-w-0">
      {!ready && <div aria-hidden data-share-skip className={cn('absolute inset-0 rounded-md bg-chip', appearance.motion !== 'none' && 'animate-pulse')} />}
      {doc ? (
        <iframe
          ref={attach}
          // Scripts only: no same-origin (the page's DOM, storage and API stay out of reach), no forms, popups or modals
          sandbox="allow-scripts"
          srcDoc={doc}
          title={title || name}
          className={cn('relative block max-h-widget w-full border-0 bg-transparent transition-opacity duration-(--dur-open) ease-out',
            height === undefined && 'h-widget-placeholder', !ready && 'opacity-0')}
          // The frame's own colour scheme matches the page inside, or Chromium paints an opaque canvas behind it
          style={{ height, colorScheme: dark ? 'dark' : 'light' }}
        />
      ) : <div className={cn('max-h-widget', height === undefined && 'h-widget-placeholder')} style={{ height }} />}
    </div>
  );
}
