import { useEffect, useRef } from 'react';
import { cn } from '../ui/cn';
import { registerPluginFrame, usePluginPiece } from '../pluginSlots';
import { digest, tokensOf } from '../panel/PluginFrame';

// What a frame hears of the session on screen (acpira.onState in /plugin-frame.html): whether it works or waits for the
// user, nothing of its content
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
  'data-skin-frame'?: string;
}

// A plugin document in the app's frame page (`/plugin-frame.html`, sandboxed without allow-same-origin, so plugin code
// never runs in the page). A new document is a new frame
export function SandboxFrame({ plugin, piece, html, options, run, state, onDrawn, focusable, title, className, ...data }: Props) {
  const ref = useRef<HTMLIFrameElement>(null);
  const drawn = useRef(onDrawn);
  drawn.current = onDrawn;
  // A piece in effect is read from the slots, so a new version of it makes a new frame too
  const found = usePluginPiece(plugin, piece);
  const version = digest(html ?? found?.html ?? '');
  const live = useRef({ run, state });
  live.current = { run, state };
  useEffect(() => {
    const el = ref.current;
    const win = el?.contentWindow;
    if (!el || !win) return;
    const css = () => tokensOf(el);
    const off = registerPluginFrame(win, plugin, piece, css, {
      ...(html !== undefined ? { html: () => html } : {}),
      options,
      // What the session does now, in the document's first load (a piece that never posts `drawn` would miss it)
      state: () => live.current.state,
      drawn: () => {
        // What changed while it loaded
        const { run, state } = live.current;
        if (run !== undefined) win.postMessage({ acpira: 'backdrop', run }, '*');
        if (state) win.postMessage({ acpira: 'state', state }, '*');
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
  }, [plugin, piece, version]);
  useEffect(() => { if (run !== undefined) ref.current?.contentWindow?.postMessage({ acpira: 'backdrop', run }, '*'); }, [run]);
  useEffect(() => { if (state) ref.current?.contentWindow?.postMessage({ acpira: 'state', state }, '*'); }, [state?.working, state?.waiting]);
  return (
    <iframe key={version} ref={ref} title={title} {...data} src={`${location.origin}/plugin-frame.html`} sandbox="allow-scripts" tabIndex={focusable ? undefined : -1}
      className={cn('block border-0 bg-transparent', className)} />
  );
}
