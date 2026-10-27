import { useEffect, useRef } from 'react';
import { cn } from '../ui/cn';
import { registerPluginFrame, usePluginPiece } from '../pluginSlots';

// The tokens a plugin's document gets on its root, read where the frame sits, so it follows the page's theme, font and
// density without seeing the page (docs/dev/plugins.md Interface)
const TOKENS = [
  '--fg-1', '--fg-2', '--fg-3', '--fg-strong', '--bg-0', '--bg-1', '--bg-2', '--bg-code', '--line', '--line-strong', '--hover',
  '--active', '--focus', '--chip', '--ok', '--warn', '--danger', '--brand', '--font', '--font-mono', '--ui-font-size',
  '--code-font-size', '--row', '--r-sm', '--r-md', '--r-lg', '--gap', '--pad',
];

function tokensOf(el: Element): string {
  const style = getComputedStyle(el);
  const light = !!el.closest('[data-theme="light"]') || document.body.classList.contains('vscode-light');
  return [`color-scheme: ${light ? 'light' : 'dark'}`, ...TOKENS.map(name => [name, style.getPropertyValue(name).trim()] as const)
    .filter(([, v]) => v).map(([name, v]) => `${name}: ${v}`)].join('; ');
}

// A short name of a document, so a new version makes a new frame without a 256 KB key
function digest(text: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 0x01000193);
  return `${text.length}-${(h >>> 0).toString(36)}`;
}

// One interface piece of a plugin: the app's frame document (`/plugin-frame.html`) in a sandbox without
// allow-same-origin, which takes the piece's HTML once it says it is ready. A new version of the piece is a new frame;
// a piece no longer in effect (switched off, safe mode) leaves nothing
export function PluginFrame({ plugin, piece, shown, className }: { plugin: string; piece: string; shown: boolean; className?: string }) {
  const found = usePluginPiece(plugin, piece);
  const ref = useRef<HTMLIFrameElement>(null);
  const html = found?.html;
  const version = html === undefined ? undefined : digest(html);
  useEffect(() => {
    const el = ref.current;
    const win = el?.contentWindow;
    if (!el || !win) return;
    const css = () => tokensOf(el);
    const off = registerPluginFrame(win, plugin, piece, css);
    // The page's look changes on attributes of the shell element and the body (light / dark from the host)
    let last = css();
    const restyle = () => { const next = css(); if (next !== last) { last = next; win.postMessage({ acpira: 'theme', css: next }, '*'); } };
    const watch = new MutationObserver(restyle);
    const shell = el.closest('.acp-shell');
    if (shell) watch.observe(shell, { attributes: true });
    watch.observe(document.body, { attributes: true, attributeFilter: ['class'] });
    return () => { watch.disconnect(); off(); };
  }, [plugin, piece, version]);
  if (version === undefined) return null;
  return (
    <iframe key={version} ref={ref} title={found?.title || plugin} src={`${location.origin}/plugin-frame.html`} sandbox="allow-scripts"
      aria-hidden={!shown || undefined} className={cn('absolute inset-0 size-full border-0 bg-transparent', !shown && 'hidden', className)} />
  );
}
