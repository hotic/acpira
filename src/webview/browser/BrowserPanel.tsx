import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { ArrowLeft, ArrowRight, ExternalLink, RotateCw, SquareDashedMousePointer, TriangleAlert } from 'lucide-react';
import { envelopeOf, normalizePreviewUrl, pickedContext, type PanelMsg, type PickedElement, type PickedRect, type PickerMsg } from '@shared/picker';
import type { BrowserHostMsg, BrowserLook, BrowserMsg } from '@shared/protocol';
import { AppearanceContext, appearanceDataAttrs, BASE_APPEARANCE } from '../appearance';
import { LocaleContext, setLocale, t } from '../i18n';
import { lookAttrs, resolveTheme, ThemeContext } from '../look';
import { useVsCodeTheme } from '../useVsCodeTheme';
import { vscodeApi } from '../vscodeApi';
import { Button, IconButton } from '../ui/Button';
import { cn } from '../ui/cn';
import { Popover, ShellLayerContext } from '../ui/Popover';
import { CommentEditor } from '../chat/Quotes';

const post = (m: BrowserMsg) => vscodeApi().postMessage(m);

interface Frame {
  target: string;
  src: string;
  token: string;
}

// How long after the frame's load event the picker's hello may still arrive (it is posted at DOMContentLoaded, so normally
// before load; the two travel separately)
const HELLO_GRACE_MS = 400;

// A proxied address as the page it stands for: same path on the target's origin. A page that left the proxy (a link to
// another port) is shown as it is
function targetOf(url: string, frame: Frame): string {
  try {
    const at = new URL(url);
    if (at.origin !== new URL(frame.src).origin) return url;
    return new URL(`${at.pathname}${at.search}${at.hash}`, new URL(frame.target).origin).toString();
  } catch {
    return url;
  }
}

// Toolbar buttons: a step the current page cannot take reads as unavailable
const TOOL = 'disabled:pointer-events-none disabled:opacity-40';

function union(rects: PickedRect[]): PickedRect | undefined {
  if (!rects.length) return undefined;
  const x = Math.min(...rects.map(r => r.x)), y = Math.min(...rects.map(r => r.y));
  const right = Math.max(...rects.map(r => r.x + r.width)), bottom = Math.max(...rects.map(r => r.y + r.height));
  return { x, y, width: right - x, height: bottom - y };
}

// The browser panel's page (window.__acpira.host === 'browser', src/host/bridge.ts BrowserPanel): an address bar over the
// proxied page and, while picking, a comment card under the picked elements. The picker inside the page does the
// hovering and selecting (src/picker); this side only steers it and turns its picks into chat context
export function BrowserPanel() {
  const [look, setLook] = useState<BrowserLook>();
  const [frame, setFrame] = useState<Frame>();
  const [error, setError] = useState<{ target: string; error: string }>();
  // What the address bar shows, and what is being typed into it
  const [shown, setShown] = useState('');
  const [typed, setTyped] = useState<string>();
  const [title, setTitle] = useState('');
  // undefined until the first page settles
  const [pickable, setPickable] = useState<boolean>();
  const [picking, setPicking] = useState(false);
  const [picked, setPicked] = useState<PickedElement[]>([]);
  const [rects, setRects] = useState<PickedRect[]>([]);
  // A new pick set gets a fresh comment field
  const [pickSeq, setPickSeq] = useState(0);
  // Bumped to reload a page the picker is not in
  const [reloads, setReloads] = useState(0);
  const iframe = useRef<HTMLIFrameElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const hellos = useRef(0);
  const checkedHellos = useRef(0);
  const pickingRef = useRef(picking);
  pickingRef.current = picking;

  const toPicker = useCallback((msg: PanelMsg) => {
    if (frame) iframe.current?.contentWindow?.postMessage({ acpiraPicker: frame.token, msg }, '*');
  }, [frame]);

  const togglePicking = (on: boolean) => {
    setPicking(on);
    toPicker({ type: 'pick', on });
    if (!on) { toPicker({ type: 'clear' }); setPicked([]); setRects([]); }
  };

  const clearPicked = () => {
    toPicker({ type: 'clear' });
    setPicked([]);
    setRects([]);
  };

  useEffect(() => {
    const onMessage = (e: MessageEvent) => {
      if (frame && e.source && e.source === iframe.current?.contentWindow) {
        const msg = envelopeOf<PickerMsg>(e.data, frame.token);
        if (msg) onPicker(msg);
        return;
      }
      const m = e.data as BrowserHostMsg | undefined;
      switch (m?.type) {
        case 'browserLook': setLook(m.look); setLocale(m.look.locale); break;
        case 'browserFrame': setFrame(m); setError(undefined); setShown(m.target); setPickable(undefined); setPicked([]); setRects([]); break;
        case 'browserError': setError(m); setShown(m.target); break;
      }
    };
    const moved = (url: string, pageTitle: string) => {
      setShown(targetOf(url, frame!));
      setTitle(pageTitle);
      post({ type: 'browserTitle', title: pageTitle });
    };
    const onPicker = (msg: PickerMsg) => {
      switch (msg.type) {
        case 'hello':
          hellos.current++;
          setPickable(true);
          setPicked([]);
          setRects([]);
          // A page that loaded while picking picks too
          if (pickingRef.current) toPicker({ type: 'pick', on: true });
          moved(msg.url, msg.title);
          break;
        case 'location': moved(msg.url, msg.title); break;
        case 'picked':
          setPicked(msg.elements);
          setRects(msg.elements.map(el => el.rect));
          setPickSeq(n => (msg.elements.length ? n : n + 1));
          break;
        case 'rects': setRects(msg.rects); break;
        case 'escape':
          if (pickedRef.current.length) clearPicked();
          else togglePicking(false);
          break;
      }
    };
    window.addEventListener('message', onMessage);
    return () => window.removeEventListener('message', onMessage);
  });
  const pickedRef = useRef(picked);
  pickedRef.current = picked;

  useEffect(() => { post({ type: 'browserReady' }); }, []);

  // A page without the picker (another site, a non-HTML response) never says hello: picking and history are off there
  const onLoad = () => {
    window.setTimeout(() => {
      setPickable(hellos.current !== checkedHellos.current);
      checkedHellos.current = hellos.current;
    }, HELLO_GRACE_MS);
  };

  const go = () => {
    if (typed === undefined) return;
    const url = normalizePreviewUrl(typed);
    if (!url) return;
    setTyped(undefined);
    setShown(url);
    post({ type: 'browserNavigate', url });
  };
  const onAddressKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.nativeEvent.isComposing) return;
    if (e.key === 'Enter') { e.preventDefault(); go(); }
    if (e.key === 'Escape') { e.preventDefault(); setTyped(undefined); e.currentTarget.blur(); }
  };
  const reload = () => {
    if (pickable) toPicker({ type: 'reload' });
    else setReloads(n => n + 1);
  };

  const add = (comment: string) => {
    if (!picked.length) return;
    const { label, text } = pickedContext(picked, shown, title);
    post({ type: 'browserAdd', item: { label, text, uri: shown, ...(comment ? { comment } : {}) } });
    togglePicking(false);
  };

  // The comment card hangs off the picked elements' outline, in this document's coordinates
  const box = union(rects);
  // A new object only when the outline moves, so the positioner follows scrolling inside the page
  const boxKey = box ? `${box.x},${box.y},${box.width},${box.height}` : '';
  const anchor = useMemo(() => {
    if (!box) return null;
    return {
      getBoundingClientRect: () => {
        const at = iframe.current?.getBoundingClientRect();
        return DOMRect.fromRect({ x: (at?.left ?? 0) + box.x, y: (at?.top ?? 0) + box.y, width: box.width, height: box.height });
      },
    };
  }, [boxKey]);

  const hostTheme = useVsCodeTheme();
  const { theme, fixed } = resolveTheme(look?.theme ?? 'auto', hostTheme);
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  const appearance = look?.appearance ?? BASE_APPEARANCE;
  const shellLook = look && { fixedTheme: fixed, uiFontSize: look.uiFontSize, codeFontSize: look.codeFontSize, diffMarkers: 'color' as const, fontSmoothing: look.fontSmoothing, sessionListPosition: 'hidden' as const };
  const address = typed ?? shown;
  const badAddress = typed !== undefined && !!typed.trim() && !normalizePreviewUrl(typed);

  return (
    <AppearanceContext.Provider value={appearance}>
      <ThemeContext.Provider value={theme}>
      <LocaleContext.Provider value={look?.locale ?? 'en'}>
        <ShellLayerContext.Provider value={root}>
          <div ref={root} className="acp-shell relative flex h-full min-h-0 w-full flex-col overflow-hidden bg-bg-0" data-theme={theme} data-surface-host="editor"
            {...appearanceDataAttrs(appearance)} {...lookAttrs(shellLook)}>
            <div className="flex shrink-0 items-center gap-1 border-b border-line px-pad py-gap-half">
              <IconButton size="sm" className={TOOL} title={t('browser.back')} aria-label={t('browser.back')} disabled={!pickable} onClick={() => toPicker({ type: 'history', go: -1 })}><ArrowLeft strokeWidth={1.5} /></IconButton>
              <IconButton size="sm" className={TOOL} title={t('browser.forward')} aria-label={t('browser.forward')} disabled={!pickable} onClick={() => toPicker({ type: 'history', go: 1 })}><ArrowRight strokeWidth={1.5} /></IconButton>
              <IconButton size="sm" className={TOOL} title={t('browser.reload')} aria-label={t('browser.reload')} disabled={!frame} onClick={reload}><RotateCw strokeWidth={1.5} /></IconButton>
              <div className={cn('mx-1 flex h-ctl-sm min-w-0 flex-1 items-center rounded-sm border border-line bg-chip px-2 focus-within:border-line-strong', badAddress && 'border-danger focus-within:border-danger')}>
                <input value={address} spellCheck={false} aria-label={t('browser.address')} aria-invalid={badAddress} placeholder={t('browser.empty')}
                  onChange={e => setTyped(e.target.value)} onKeyDown={onAddressKey} onBlur={() => { if (!badAddress) setTyped(undefined); }}
                  className="min-w-0 flex-1 border-0 bg-transparent text-3 text-fg-1 outline-none placeholder:text-fg-3" />
              </div>
              <IconButton size="sm" title={pickable === false ? t('browser.notPickable') : t('browser.pick')} aria-label={t('browser.pick')} aria-pressed={picking}
                disabled={!pickable} onClick={() => togglePicking(!picking)} className={cn(TOOL, picking && 'bg-active text-fg-1')}>
                <SquareDashedMousePointer strokeWidth={1.5} />
              </IconButton>
              <IconButton size="sm" className={TOOL} title={t('browser.openExternal')} aria-label={t('browser.openExternal')} disabled={!shown} onClick={() => post({ type: 'browserOpenExternal', url: shown })}><ExternalLink strokeWidth={1.5} /></IconButton>
            </div>
            {pickable === false && frame && (
              <div className="flex h-row shrink-0 items-center gap-1 border-b border-line px-pad text-3 text-fg-3">
                <TriangleAlert strokeWidth={1.5} className="size-icon shrink-0" />
                <span className="truncate">{t('browser.notPickable')}</span>
              </div>
            )}
            <div className="relative min-h-0 flex-1">
              {error ? (
                <div className="flex h-full flex-col items-center justify-center gap-gap px-page text-center">
                  <p className="text-2 text-fg-2 break-words">{error.error === 'engine-too-old' ? t('browser.engineTooOld') : t('browser.failed', { url: error.target, error: error.error })}</p>
                  {error.error !== 'engine-too-old' && <Button onClick={() => post({ type: 'browserNavigate', url: error.target })}>{t('browser.retry')}</Button>}
                </div>
              ) : frame && (
                // Pages expect an opaque canvas: one that paints nothing would show the editor theme through
                <iframe key={`${frame.src}#${reloads}`} ref={iframe} src={frame.src} title={title || shown} onLoad={onLoad}
                  className="absolute inset-0 size-full border-0 bg-web-page" />
              )}
            </div>
            <Popover.Root open={!!anchor && picked.length > 0} onOpenChange={(next, details) => { if (!next && details.reason === 'escape-key') clearPicked(); }}>
              <Popover.Portal>
                <Popover.Positioner side="bottom" align="start" width="lg" anchor={anchor}>
                  <Popover.Popup className="flex flex-col gap-1 p-1">
                    <div className="flex min-w-0 items-center gap-1 px-1 text-3 text-fg-3">
                      <SquareDashedMousePointer strokeWidth={1.5} className="size-icon shrink-0" />
                      <span className="truncate font-mono">{picked.length > 1 ? t('browser.picked', { n: picked.length }) : picked[0]?.selector}</span>
                    </div>
                    {/* Enter adds, with or without a comment; Escape drops the pick */}
                    <CommentEditor key={pickSeq} onSave={comment => add(comment)} onCancel={clearPicked} />
                    <span className="px-1 text-3 text-fg-3">{t('browser.addHint')}</span>
                  </Popover.Popup>
                </Popover.Positioner>
              </Popover.Portal>
            </Popover.Root>
          </div>
        </ShellLayerContext.Provider>
      </LocaleContext.Provider>
      </ThemeContext.Provider>
    </AppearanceContext.Provider>
  );
}
