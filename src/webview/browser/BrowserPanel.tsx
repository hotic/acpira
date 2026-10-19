import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeft, ArrowRight, ExternalLink, MessageSquarePlus, MessageSquareText, RotateCw, SquareDashedMousePointer, TriangleAlert } from 'lucide-react';
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
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';

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

// The pick toolbar's two actions, like the transcript quote's "Add to chat"
const PICK_ACTION = 'inline-flex h-ctl-sm items-center gap-1 whitespace-nowrap rounded-sm px-2 text-3 font-medium text-fg-1 outline-none transition-colors hover:bg-active focus-visible:bg-active [&_svg]:size-icon [&_svg]:text-fg-3';
const MOD_KEY = /Mac|iPhone|iPad/.test(navigator.platform) ? '⌘' : 'Ctrl+';

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
  // "Comment to chat" swapped the pick's toolbar for a comment field
  const [commenting, setCommenting] = useState(false);
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

  // The outlines in the page take the panel's accent
  const pickMsg = (on: boolean): PanelMsg => {
    const css = root.current ? getComputedStyle(root.current) : undefined;
    const accent = css?.getPropertyValue('--accent').trim(), accentFg = css?.getPropertyValue('--accent-fg').trim();
    return { type: 'pick', on, ...(accent ? { accent } : {}), ...(accentFg ? { accentFg } : {}) };
  };

  const togglePicking = (on: boolean) => {
    setCommenting(false);
    setPicking(on);
    toPicker(pickMsg(on));
    if (!on) { toPicker({ type: 'clear' }); setPicked([]); setRects([]); }
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
          if (pickingRef.current) toPicker(pickMsg(true));
          moved(msg.url, msg.title);
          break;
        case 'location': moved(msg.url, msg.title); break;
        case 'picked':
          setCommenting(false);
          setPicked(msg.elements);
          setRects(msg.elements.map(el => el.rect));
          break;
        case 'rects': setRects(msg.rects); break;
        // Keys pressed while the page has focus
        case 'escape': togglePicking(false); break;
        case 'add': add(''); break;
        case 'comment': if (pickedRef.current.length) setCommenting(true); break;
      }
    };
    window.addEventListener('message', onMessage);
    return () => window.removeEventListener('message', onMessage);
  });

  const pickedRef = useRef(picked);
  pickedRef.current = picked;

  // The same keys with focus in the panel itself (the pick toggle was just clicked), which the page never sees: Escape
  // leaves picking, Enter adds the pick, Cmd / Ctrl+J comments on it. Fields with keys of their own (address bar, comment)
  // stop them first
  useEffect(() => {
    if (!picking) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing || commenting) return;
      if ((e.target as Element | null)?.closest?.('input, textarea')) return;
      if (e.key === 'Escape') { e.preventDefault(); togglePicking(false); return; }
      if (!picked.length) return;
      if (e.key === 'Enter') { e.preventDefault(); add(''); }
      else if (e.key.toLowerCase() === 'j' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); setCommenting(true); }
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  });

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
  const onAddressKey = (e: ReactKeyboardEvent<HTMLInputElement>) => {
    if (e.nativeEvent.isComposing) return;
    if (e.key === 'Enter') { e.preventDefault(); go(); }
    if (e.key === 'Escape') { e.preventDefault(); setTyped(undefined); e.currentTarget.blur(); }
  };
  const reload = () => {
    if (pickable) toPicker({ type: 'reload' });
    else setReloads(n => n + 1);
  };

  const add = (comment: string) => {
    const elements = pickedRef.current;
    if (!elements.length) return;
    const { label, text } = pickedContext(elements, shown, title);
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
            {/* Under the pick: "Comment to chat" (Cmd / Ctrl+J, a comment field with a confirm button) and "Add to chat" (Enter) */}
            <Popover.Root open={!!anchor && picked.length > 0} onOpenChange={(next, details) => { if (!next && details.reason === 'escape-key') togglePicking(false); }}>
              <Popover.Portal>
                <Popover.Positioner side="bottom" align="start" width={commenting ? 'lg' : 'anchor'} anchor={anchor}
                  className={cn('pointer-events-auto', !commenting && 'w-auto')}>
                  <Popover.Popup className={cn(!commenting && 'flex-row items-center p-0.5')}>
                    {commenting
                      ? <CommentEditor className="w-full" submit={t('browser.addComment')} onSave={comment => add(comment)} onCancel={() => setCommenting(false)} />
                      : <>
                          <button type="button" onClick={() => setCommenting(true)} className={PICK_ACTION}>
                            <MessageSquareText strokeWidth={1.5} />
                            {t('browser.comment')}
                            <span className="text-fg-3">{MOD_KEY}J</span>
                          </button>
                          <span aria-hidden className="mx-0.5 h-icon w-px shrink-0 bg-line" />
                          <button type="button" onClick={() => add('')} className={PICK_ACTION}>
                            <MessageSquarePlus strokeWidth={1.5} />
                            {picked.length > 1 ? t('browser.addCount', { n: picked.length }) : t('quote.add')}
                            <span className="text-fg-3">↵</span>
                          </button>
                        </>}
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
