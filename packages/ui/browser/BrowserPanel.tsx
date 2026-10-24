import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeft, ArrowRight, ExternalLink, Globe, MessageSquarePlus, MessageSquareText, RotateCw, SquareDashedMousePointer, TriangleAlert } from 'lucide-react';
import { envelopeOf, MAX_CONTEXT_CALL_ID, normalizePreviewUrl, pickedContext, redactUrl, type PanelMsg, type PickedElement, type PickedRect, type PickerMsg } from '@acpira/shared/picker';
import type { BrowserHostMsg, BrowserLook, BrowserMsg, BrowserPage } from '@acpira/shared/protocol';
import { optionClass } from '../ui/DropdownMenu';
import { popupClass } from '../ui/Overlay';
import { AppearanceContext, appearanceDataAttrs, BASE_APPEARANCE } from '../appearance';
import { LocaleContext, setLocale, t } from '../i18n';
import { lookAttrs, resolveTheme, ThemeContext } from '../look';
import { useVsCodeTheme } from '../useVsCodeTheme';
import { vscodeApi } from '../vscodeApi';
import { Button, IconButton } from '../ui/Button';
import { cn } from '../ui/cn';
import { Popover, ShellLayerContext } from '../ui/Popover';
import { useMorph } from '../ui/useMorph';
import { CommentEditor } from '../chat/Quotes';
import { desktopShell } from '../desktop';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';

const post = (m: BrowserMsg) => vscodeApi().postMessage(m);
// The page's "open elsewhere" button: VS Code's own browser in the extension, the system browser in the desktop app
const externalLabel = desktopShell() ? 'browser.openSystem' : 'browser.openExternal';

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
const MAX_SUGGESTIONS = 8;

// Toolbar buttons: a step the current page cannot take reads as unavailable
const TOOL = 'disabled:pointer-events-none disabled:opacity-40';

// The shell's messages come from the webview host, which shares this document's origin. `e.source === window.parent` cannot
// tell: VS Code's webview script sets `window.parent = window` (and `top`), so the outer frame that relays them is never
// `window.parent` (verified on VS Code 1.142.0). The page's own frames can post here too (a nested iframe is not the
// picker's window), but they are served from the proxy's http origin, so anything from another origin is dropped, and a
// message is used only in the shape it has
const str = (v: unknown): v is string => typeof v === 'string';
// A browser tab of the desktop tool panel (apps/desktop/src/page.ts names the tab), not a window of its own. A tab on the files
// layout's stage (`stage` in its address, panel/ToolPanel.tsx BrowserFrame) is the stage's own sheet, so it keeps the plain
// surface of a window instead of lying on the canvas
const IN_PANEL = typeof window !== 'undefined' && !!window.__acpira?.tab && !new URLSearchParams(location.search).has('stage');

function hostMsgOf(e: MessageEvent): BrowserHostMsg | undefined {
  if (e.origin !== window.origin) return undefined;
  const m = e.data as Partial<Record<string, unknown>> | null;
  switch (m?.type) {
    case 'browserFrame': return str(m.target) && str(m.src) && str(m.token) ? m as BrowserHostMsg : undefined;
    case 'browserError': return str(m.target) && str(m.error) ? m as BrowserHostMsg : undefined;
    case 'browserLook': return m.look && typeof m.look === 'object' ? m as BrowserHostMsg : undefined;
    case 'browserHistory':
      return Array.isArray(m.pages) && m.pages.every(p => p && typeof p === 'object' && str((p as BrowserPage).url) && str((p as BrowserPage).title)) ? m as BrowserHostMsg : undefined;
    case 'browserContextDone':
      return str(m.id) && (m.error === undefined || str(m.error)) && (m.shown === undefined || typeof m.shown === 'boolean') ? m as BrowserHostMsg : undefined;
  }
  return undefined;
}

// A page's window.acpira.context call as the shell may see it. The page can post anything with the token it was served (its
// own scripts can read the picker's URL), so only the call's frame is checked here; the payload is the shell's to parse
function contextCallOf(msg: Extract<PickerMsg, { type: 'context' }>): BrowserMsg | undefined {
  if (!str(msg.id) || !msg.id || msg.id.length > MAX_CONTEXT_CALL_ID || (msg.op !== 'set' && msg.op !== 'add')) return undefined;
  return { type: 'browserContext', id: msg.id, op: msg.op, payload: msg.payload };
}

// The clears a leaving page posts from pagehide (packages/picker) arrive once its document is gone, so with no source to match
// against the frame (seen in Chrome when page and panel share a process). Taken only as what they are: a `set` with no
// items, stamped with the frame's token, from the frame's origin
function leavingClearOf(e: MessageEvent, frame: Frame): BrowserMsg | undefined {
  let origin: string;
  try { origin = new URL(frame.src, location.href).origin; } catch { return undefined; }
  if (e.source !== null || e.origin !== origin) return undefined;
  const msg = envelopeOf<PickerMsg>(e.data, frame.token);
  if (msg?.type !== 'context' || msg.op !== 'set') return undefined;
  const items = (msg.payload as { items?: unknown } | null | undefined)?.items;
  return Array.isArray(items) && !items.length ? contextCallOf(msg) : undefined;
}

function union(rects: PickedRect[]): PickedRect | undefined {
  if (!rects.length) return undefined;
  const x = Math.min(...rects.map(r => r.x)), y = Math.min(...rects.map(r => r.y));
  const right = Math.max(...rects.map(r => r.x + r.width)), bottom = Math.max(...rects.map(r => r.y + r.height));
  return { x, y, width: right - x, height: bottom - y };
}

// The browser panel's page (window.__acpira.host === 'browser', apps/vscode/src/bridge.ts BrowserPanel): an address bar over the
// proxied page and, while picking, a comment card under the picked elements. The picker inside the page does the
// hovering and selecting (packages/picker); this side only steers it, turns its picks into chat context and passes the page's own
// context calls (window.acpira.context) on to the shell
export function BrowserPanel() {
  const [look, setLook] = useState<BrowserLook>();
  const [frame, setFrame] = useState<Frame>();
  const [error, setError] = useState<{ target: string; error: string }>();
  // What the address bar shows, and what is being typed into it
  const [shown, setShown] = useState('');
  const [typed, setTyped] = useState<string>();
  // Recent pages (from the shell) offered under the address bar while it has focus; `hl` is the arrow-key highlight
  const [history, setHistory] = useState<BrowserPage[]>([]);
  const [addressFocused, setAddressFocused] = useState(false);
  const [hl, setHl] = useState(-1);
  const addressInput = useRef<HTMLInputElement>(null);
  const [title, setTitle] = useState('');
  // undefined until the first page settles
  const [pickable, setPickable] = useState<boolean>();
  const [picking, setPicking] = useState(false);
  const [picked, setPicked] = useState<PickedElement[]>([]);
  const [rects, setRects] = useState<PickedRect[]>([]);
  // "Comment to chat" swapped the pick's toolbar for a comment field
  const [commenting, setCommenting] = useState(false);
  const morph = useMorph<HTMLDivElement>(commenting);
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
      const clear = frame && leavingClearOf(e, frame);
      if (clear) { post(clear); return; }
      const m = hostMsgOf(e);
      switch (m?.type) {
        case 'browserLook': setLook(m.look); setLocale(m.look.locale); break;
        case 'browserFrame': setFrame(m); setError(undefined); setShown(m.target); setPickable(undefined); setPicked([]); setRects([]); break;
        case 'browserError': setError(m); setShown(m.target); break;
        case 'browserHistory': setHistory(m.pages); break;
        // The shell's answer to the page's context call, back to the page that is showing (a page that left drops it)
        case 'browserContextDone': toPicker({ type: 'contextDone', id: m.id, ...(m.error ? { error: m.error } : {}), ...(m.shown === undefined ? {} : { shown: m.shown }) }); break;
      }
    };
    const moved = (url: string, pageTitle: string) => {
      setShown(targetOf(url, frame!));
      setTitle(pageTitle);
      post({ type: 'browserTitle', url: targetOf(url, frame!), title: pageTitle });
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
        // The page's own window.acpira.context call, on to the shell
        case 'context': {
          const call = contextCallOf(msg);
          if (call) post(call);
          break;
        }
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
  // The address VS Code hands back after a window reload (apps/vscode/src/extension.ts deserializes on BROWSER_STATE_URL)
  useEffect(() => {
    const api = vscodeApi();
    if (normalizePreviewUrl(shown)) api.setState?.({ acpiraBrowserUrl: shown });
  }, [shown]);

  // A page without the picker (another site, a non-HTML response) never says hello: picking and history are off there
  const onLoad = () => {
    window.setTimeout(() => {
      setPickable(hellos.current !== checkedHellos.current);
      checkedHellos.current = hellos.current;
    }, HELLO_GRACE_MS);
  };

  const open = (raw: string) => {
    const url = normalizePreviewUrl(raw);
    if (!url) return;
    setTyped(undefined);
    setShown(url);
    setHl(-1);
    addressInput.current?.blur();
    post({ type: 'browserNavigate', url });
  };
  // Recent pages matching what is typed (all of them before anything is), at most a short list
  const query = (typed ?? '').trim().toLowerCase();
  const suggestions = history.filter(p => !query || p.url.toLowerCase().includes(query) || p.title.toLowerCase().includes(query)).slice(0, MAX_SUGGESTIONS);
  const suggesting = addressFocused && suggestions.length > 0 && (typed !== undefined || !frame);
  const onAddressKey = (e: ReactKeyboardEvent<HTMLInputElement>) => {
    if (e.nativeEvent.isComposing) return;
    if (e.key === 'ArrowDown' && suggesting) { e.preventDefault(); setHl(i => Math.min(i + 1, suggestions.length - 1)); }
    if (e.key === 'ArrowUp' && suggesting) { e.preventDefault(); setHl(i => Math.max(i - 1, -1)); }
    if (e.key === 'Enter') { e.preventDefault(); open(suggesting && hl >= 0 ? suggestions[hl]!.url : typed ?? shown); }
    if (e.key === 'Escape') { e.preventDefault(); setTyped(undefined); setHl(-1); e.currentTarget.blur(); }
  };
  // An empty panel starts on its address bar
  useEffect(() => { if (!frame && !error) addressInput.current?.focus(); }, [frame, error]);
  const reload = () => {
    if (pickable) toPicker({ type: 'reload' });
    else setReloads(n => n + 1);
  };

  const add = (comment: string) => {
    const elements = pickedRef.current;
    if (!elements.length) return;
    const { label, text } = pickedContext(elements);
    // The address goes to the model as the pick's source: secret-looking query parameters are redacted like the page's own URLs
    post({ type: 'browserAdd', item: { label, text, uri: redactUrl(shown), ...(comment ? { comment } : {}) } });
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
          {/* A tab of the desktop tool panel lies on the window canvas like the Files tab, and a page shown is a sheet on it
              (desktop.css [data-sheet]); a window or an editor tab of its own keeps the plain surface */}
          <div ref={root} className={cn('acp-shell relative flex h-full min-h-0 w-full flex-col overflow-hidden', !IN_PANEL && 'bg-bg-0')} data-theme={theme} data-surface-host="editor"
            data-desktop={IN_PANEL || undefined} {...appearanceDataAttrs(appearance)} {...lookAttrs(shellLook)}>
            <div className={cn('flex shrink-0 items-center gap-1 px-pad py-gap-half', !IN_PANEL && 'border-b border-line')}>
              <IconButton size="sm" className={TOOL} title={t('browser.back')} aria-label={t('browser.back')} disabled={!pickable} onClick={() => toPicker({ type: 'history', go: -1 })}><ArrowLeft strokeWidth={1.5} /></IconButton>
              <IconButton size="sm" className={TOOL} title={t('browser.forward')} aria-label={t('browser.forward')} disabled={!pickable} onClick={() => toPicker({ type: 'history', go: 1 })}><ArrowRight strokeWidth={1.5} /></IconButton>
              <IconButton size="sm" className={TOOL} title={t('browser.reload')} aria-label={t('browser.reload')} disabled={!frame} onClick={reload}><RotateCw strokeWidth={1.5} /></IconButton>
              <div className={cn('relative mx-1 flex h-ctl-sm min-w-0 flex-1 items-center rounded-sm border border-line bg-chip px-2 focus-within:border-line-strong', badAddress && 'border-danger focus-within:border-danger')}>
                <input ref={addressInput} value={address} spellCheck={false} aria-label={t('browser.address')} aria-invalid={badAddress} placeholder={t('browser.empty')}
                  role="combobox" aria-expanded={suggesting} aria-autocomplete="list"
                  onChange={e => { setTyped(e.target.value); setHl(-1); }} onKeyDown={onAddressKey}
                  onFocus={e => { setAddressFocused(true); e.currentTarget.select(); }}
                  onBlur={() => { setAddressFocused(false); setHl(-1); if (!badAddress) setTyped(undefined); }}
                  className="min-w-0 flex-1 border-0 bg-transparent text-3 text-fg-1 outline-none placeholder:text-fg-3" />
                {suggesting && (
                  <div role="listbox" className={cn(popupClass, 'absolute inset-x-0 top-full z-30 mt-1')} data-palette="widget">
                    {suggestions.map((p, i) => (
                      // Pressing keeps the caret in the address bar until the click picks the page
                      <div key={p.url} role="option" aria-selected={i === hl} data-highlighted={i === hl ? '' : undefined}
                        onMouseDown={e => e.preventDefault()} onMouseEnter={() => setHl(i)} onClick={() => open(p.url)}
                        className={cn(optionClass, 'cursor-default')}>
                        <Globe strokeWidth={1.5} className="size-icon shrink-0 text-fg-3" />
                        <span className="min-w-0 shrink truncate">{p.title || p.url}</span>
                        {p.title && <span className="min-w-0 flex-1 truncate text-3 text-fg-3">{p.url}</span>}
                        {i === 0 && !query && <span className="ml-auto shrink-0 text-3 text-fg-3">{t('browser.recent')}</span>}
                      </div>
                    ))}
                  </div>
                )}
              </div>
              <IconButton size="sm" title={pickable === false ? t('browser.notPickable') : t('browser.pick')} aria-label={t('browser.pick')} aria-pressed={picking}
                disabled={!pickable} onClick={() => togglePicking(!picking)} className={cn(TOOL, picking && 'bg-active text-fg-1')}>
                <SquareDashedMousePointer strokeWidth={1.5} />
              </IconButton>
              <IconButton size="sm" className={TOOL} title={t(externalLabel)} aria-label={t(externalLabel)} disabled={!shown} onClick={() => post({ type: 'browserOpenExternal', url: shown })}><ExternalLink strokeWidth={1.5} /></IconButton>
            </div>
            {pickable === false && frame && (
              <div className="flex h-row shrink-0 items-center gap-1 border-b border-line px-pad text-3 text-fg-3">
                <TriangleAlert strokeWidth={1.5} className="size-icon shrink-0" />
                <span className="truncate">{t('browser.notPickable')}</span>
              </div>
            )}
            <div data-sheet={(IN_PANEL && frame && !error) || undefined} className="relative min-h-0 flex-1">
              {error ? (
                <div className="flex h-full flex-col items-center justify-center gap-gap px-page text-center">
                  <p className="text-2 text-fg-2 break-words">{error.error === 'engine-too-old' ? t('browser.engineTooOld') : t('browser.failed', { url: error.target, error: error.error })}</p>
                  {error.error !== 'engine-too-old' && <Button onClick={() => post({ type: 'browserNavigate', url: error.target })}>{t('browser.retry')}</Button>}
                </div>
              ) : frame ? (
                // Pages expect an opaque canvas: one that paints nothing would show the editor theme through
                <iframe key={`${frame.src}#${reloads}`} ref={iframe} src={frame.src} title={title || shown} onLoad={onLoad}
                  className="absolute inset-0 size-full border-0 bg-web-page" />
              ) : !shown && (
                <div className="flex h-full flex-col items-center justify-center gap-1 px-page text-center">
                  <Globe strokeWidth={1} className="mb-gap size-ctl text-fg-3" />
                  <p className="m-0 text-2 font-medium text-fg-1">{t('browser.source')}</p>
                  <p className="m-0 text-2 text-fg-3">{t('browser.emptyHint')}</p>
                </div>
              )}
            </div>
            {/* Under the pick: "Comment" (Cmd / Ctrl+J; grows into a comment field with a confirm button) and "Add" (Enter, a double click) */}
            <Popover.Root open={!!anchor && picked.length > 0} onOpenChange={(next, details) => { if (!next && details.reason === 'escape-key') togglePicking(false); }}>
              <Popover.Portal>
                <Popover.Positioner side="bottom" align="start" width={commenting ? 'lg' : 'anchor'} anchor={anchor}
                  className={cn('pointer-events-auto', !commenting && 'w-auto')}>
                  <Popover.Popup ref={morph} className={cn(!commenting && 'flex-row items-center p-0.5')}>
                    {commenting
                      ? <CommentEditor className="w-full bg-transparent" submit={t('browser.addComment')} onSave={comment => add(comment)} onCancel={() => setCommenting(false)} />
                      : <>
                          <button type="button" onClick={() => setCommenting(true)} className={PICK_ACTION}>
                            <MessageSquareText strokeWidth={1.5} />
                            {t('browser.comment')}
                          </button>
                          <span aria-hidden className="mx-0.5 h-icon w-px shrink-0 bg-line" />
                          <button type="button" onClick={() => add('')} className={PICK_ACTION}>
                            <MessageSquarePlus strokeWidth={1.5} />
                            {t('browser.add')}
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
