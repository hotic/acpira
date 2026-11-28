import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { FRAME_KIT } from '../chrome/pluginKit';
import { desktopShell } from '../desktop';
import { cn } from '../ui/cn';
import {
  RUNTIME_MARK,
  cachedPluginRuntime,
  currentSessionOutline,
  loadPluginRuntime,
  registerPluginFrame,
  usePluginPiece,
} from '../pluginSlots';
import { buildPluginFrameDoc, digest, pluginPageNonce, tokensOf } from './pluginFrameDoc';

// What a frame hears of the session on screen (acpira.onState in /plugin-frame.html or srcdoc): whether it works or
// waits for the user, nothing of its content
export interface FrameState { working: boolean; waiting: boolean }

interface Props {
  plugin: string;
  // Unique among the plugin's frames on the page
  piece: string;
  // The document, when the page holds it (a backdrop); else the plugin's piece of that id in effect
  html?: string;
  options?: unknown;
  // Paused or running, for a backdrop (`{ acpira: 'backdrop', run }`)
  run?: boolean;
  state?: FrameState;
  onDrawn?: () => void;
  // A piece the user works with takes focus with Tab; a backdrop or a decoration does not
  focusable?: boolean;
  title: string;
  className?: string;
  'aria-hidden'?: boolean;
  'data-skin-frame'?: string;
}

let cachedNonce: string | undefined;

// A plugin document in a sandboxed iframe without allow-same-origin, so plugin code never runs in the page. On desktop
// the frame loads `/plugin-frame.html`; on other webview hosts (VS Code, IntelliJ, browser harness) it runs as a
// nonce-tagged `srcDoc` built with `buildPluginFrameDoc`. A new document is a new frame
export function SandboxFrame({ plugin, piece, html, options, run, state, onDrawn, focusable, title, className, ...data }: Props) {
  const ref = useRef<HTMLIFrameElement>(null);
  const drawn = useRef(onDrawn);
  drawn.current = onDrawn;
  const inline = !desktopShell();
  // A piece in effect is read from the slots, so a new version of it makes a new frame too
  const found = usePluginPiece(plugin, piece);
  const sourceHtml = html ?? found?.html;
  const snapshot = html === undefined && found?.slot === 'page' ? found.snapshot : undefined;
  const version = snapshot ?? digest(sourceHtml ?? '');
  const live = useRef({ run, state, options });
  live.current = { run, state, options };

  const wantsLib = inline && !!sourceHtml && sourceHtml.includes(RUNTIME_MARK);
  const [lib, setLib] = useState<string | null | undefined>(() => (wantsLib ? cachedPluginRuntime() : null));
  useEffect(() => {
    if (!wantsLib) { setLib(null); return; }
    const cached = cachedPluginRuntime();
    if (cached !== undefined) { setLib(cached); return; }
    let alive = true;
    loadPluginRuntime().then(
      text => { if (alive) setLib(text); },
      () => { if (alive) setLib(null); },
    );
    return () => { alive = false; };
  }, [wantsLib]);

  const doc = useMemo(() => {
    if (!inline || sourceHtml === undefined || (wantsLib && lib === undefined)) return undefined;
    const hostEl = typeof document !== 'undefined' ? document.querySelector('.acp-shell') ?? document.documentElement : undefined;
    const css = hostEl ? tokensOf(hostEl) : '';
    return buildPluginFrameDoc(sourceHtml, {
      nonce: cachedNonce ??= pluginPageNonce(),
      css,
      kit: FRAME_KIT,
      lib: lib ?? undefined,
      options: live.current.options,
      state: live.current.state,
      session: html === undefined ? currentSessionOutline() : undefined,
    });
    // Live run, state and theme changes are posted to the running frame below rather than reloading it
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [inline, sourceHtml, wantsLib, lib, version, html === undefined]);

  // Layout effect registers the frame before a srcDoc iframe parses and runs its scripts, so early calls never race
  useLayoutEffect(() => {
    const el = ref.current;
    const win = el?.contentWindow;
    if (!el || !win) return;
    const css = () => tokensOf(el);
    const syncLive = () => {
      const { run, state } = live.current;
      if (run !== undefined) win.postMessage({ acpira: 'backdrop', run }, '*');
      if (state) win.postMessage({ acpira: 'state', state }, '*');
    };
    const off = registerPluginFrame(win, plugin, piece, css, {
      ...(html !== undefined ? { html: () => html } : {}),
      options: live.current.options,
      snapshot,
      inline,
      // What the session does now, in the document's first load (a piece that never posts `drawn` would miss it)
      state: () => live.current.state,
      ready: inline ? syncLive : undefined,
      drawn: () => {
        // What changed while it loaded
        syncLive();
        drawn.current?.();
      },
    });
    let last = css();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const restyle = () => { timer = undefined; const next = css(); if (next !== last) { last = next; win.postMessage({ acpira: 'theme', css: next }, '*'); } };
    // A drag writes the shell's style attribute every frame (its width): that only settles the look once it stops
    const watch = new MutationObserver(records => {
      if (records.every(r => r.type === 'attributes' && r.attributeName === 'style')) { clearTimeout(timer); timer = setTimeout(restyle, 150); } else restyle();
    });
    const shell = el.closest('.acp-shell');
    if (shell) watch.observe(shell, { attributes: true });
    watch.observe(document.body, { attributes: true, attributeFilter: ['class'] });
    // The plugins' theme layer is a style element of its own
    watch.observe(document.head, { childList: true, subtree: true, characterData: true });
    return () => { clearTimeout(timer); watch.disconnect(); off(); };
    // A new document or another piece makes the frame anew; the rest is posted to it below
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plugin, piece, version, doc]);

  useEffect(() => { if (run !== undefined) ref.current?.contentWindow?.postMessage({ acpira: 'backdrop', run }, '*'); }, [run]);
  useEffect(() => { if (state) ref.current?.contentWindow?.postMessage({ acpira: 'state', state }, '*'); }, [state?.working, state?.waiting]);

  if (inline) {
    if (doc === undefined) return null;
    return (
      <iframe key={version} ref={ref} title={title} {...data} srcDoc={doc} sandbox="allow-scripts" tabIndex={focusable ? undefined : -1}
        className={cn('block border-0 bg-transparent', className)} />
    );
  }
  return (
    <iframe key={version} ref={ref} title={title} {...data} src={`${location.origin}/plugin-frame.html`} sandbox="allow-scripts" tabIndex={focusable ? undefined : -1}
      className={cn('block border-0 bg-transparent', className)} />
  );
}
