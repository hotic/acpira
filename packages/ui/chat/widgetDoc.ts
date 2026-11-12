// The document an HTML widget (Acpira's `show_widget`) runs as: the agent's page behind a head of our own, loaded into a
// sandboxed srcdoc frame (`AgentWidget.tsx`). Pure string work, so it is tested without a DOM.
//
// Why it is built this way (checked in Chrome 2026-10-22 against the VS Code CSP): a srcdoc, data: or blob: frame
// inherits the page's CSP, so inline scripts only run with the page's nonce. Every <script> of the widget gets it, and a
// second, stricter CSP of our own leaves the frame no network at all (fetch, images, fonts from anywhere but data:).
// The frame has `sandbox="allow-scripts"` without same-origin, so it cannot reach the page's DOM, storage or API either.
//
// Our head comes first. The widget's own `<!doctype>`, `<html>` and `<head>` that follow are merged by the HTML parser
// (a second <head> is ignored and its meta / style / script children still land in the head), so a full page and a
// bare fragment both work.

// Host token → the variable a widget reads, in the naming models already write (shadcn-style `bg-card`,
// `text-muted-foreground`, …). Read from the frame's own element, so surface and accent settings apply
export const WIDGET_VARS: Record<string, string> = {
  background: '--bg-0',
  foreground: '--fg-1',
  card: '--card-bg',
  'card-foreground': '--fg-1',
  popover: '--card-bg',
  'popover-foreground': '--fg-1',
  muted: '--chip',
  'muted-foreground': '--fg-2',
  secondary: '--chip',
  'secondary-foreground': '--fg-1',
  accent: '--chip-hover',
  'accent-foreground': '--fg-1',
  primary: '--accent',
  'primary-foreground': '--accent-fg',
  border: '--line',
  input: '--line-strong',
  ring: '--focus',
  success: '--ok',
  warning: '--warn',
  danger: '--danger',
  destructive: '--danger',
  'font-sans': '--font',
  'font-mono': '--font-mono',
  'font-size': '--text-1',
  radius: '--r-md',
};

// The Tailwind theme over those variables: colour utilities for each colour name, and `dark:` following the editor theme
// (a `dark` class on <html>) instead of the system setting. The fonts need no mapping: Tailwind's own --font-sans /
// --font-mono sit in its theme layer, which the unlayered :root rule below overrides
const COLOR_NAMES = Object.keys(WIDGET_VARS).filter(k => !k.startsWith('font-') && k !== 'radius');
const TAILWIND_THEME = `@custom-variant dark (&:where(.dark, .dark *));
@theme inline {
${COLOR_NAMES.map(n => `  --color-${n}: var(--${n});`).join('\n')}
}`;

export interface WidgetTheme {
  vars: Record<string, string>;
  dark: boolean;
}

// Messages between the frame and the page. The frame only accepts the page's (`event.source === parent`), the page only
// the frame's own window
export const WIDGET_MESSAGE = 'acpiraWidget';
// The longest text `acpira.sendPrompt` takes from a widget; longer is refused in the frame and dropped by the page
export const WIDGET_PROMPT_MAX = 20_000;
// `prompt`: the widget's `acpira.sendPrompt(text)`, sent as the user's next message (AgentWidget.tsx checks it came from a click)
// `snapshot`: a screenshot of the reply asks the page to draw itself (the sandbox keeps the frame unreadable from outside);
// it answers with a PNG data URL, or null when it could not draw
export type WidgetFrameMsg = { [WIDGET_MESSAGE]: 'height'; height: number } | { [WIDGET_MESSAGE]: 'prompt'; text: string }
  | { [WIDGET_MESSAGE]: 'snapshot'; id: string; png: string | null };
export type WidgetPageMsg = ({ [WIDGET_MESSAGE]: 'theme' } & WidgetTheme) | { [WIDGET_MESSAGE]: 'snapshot'; id: string; dpr: number };

// Runs first in the frame. It reports the content height (the page sizes the frame to it), applies theme updates, and
// turns inline event handlers into nonce'd scripts: `onclick="…"` attributes are blocked by the CSP, so each one is
// removed before it can fire and rebound through addEventListener, also on elements added later (innerHTML).
// Links are kept from navigating the frame. `window.acpira.sendPrompt(text)` is the widget's way back to the agent: it
// only goes out while the frame holds a user activation (a click, a key), so a widget cannot talk on its own at load.
const BOOTSTRAP = `(function () {
  var N = document.currentScript.nonce, KEY = ${JSON.stringify(WIDGET_MESSAGE)}, els = window.__acpiraOn = [];
  window.acpira = Object.freeze({
    sendPrompt: function (text) {
      if (typeof text !== 'string' || !text.trim()) throw new TypeError('acpira.sendPrompt: text must be a non-empty string');
      if (text.length > ${WIDGET_PROMPT_MAX}) throw new RangeError('acpira.sendPrompt: text is longer than ${WIDGET_PROMPT_MAX} characters');
      if (navigator.userActivation && !navigator.userActivation.isActive) throw new Error('acpira.sendPrompt: call it from a click or key handler');
      parent.postMessage({ [KEY]: 'prompt', text: text }, '*');
    },
  });
  var WIN = /^(load|resize|scroll|hashchange|popstate|message|pageshow|pagehide|beforeunload|unload)$/;
  function rebind(el) {
    if (!el.attributes) return;
    var found = [];
    for (var i = el.attributes.length - 1; i >= 0; i--) {
      var a = el.attributes[i];
      if (/^on[a-z]+$/i.test(a.name)) { found.push([a.name.slice(2).toLowerCase(), a.value]); el.removeAttribute(a.name); }
    }
    if (!found.length) return;
    var k = els.push(el) - 1, src = '';
    for (var j = 0; j < found.length; j++) {
      var type = found[j][0], target = el === document.body && WIN.test(type) ? 'window' : 'el';
      src += target + '.addEventListener(' + JSON.stringify(type) + ', function (event) { var r = (function (event) {\\n' + found[j][1] +
        '\\n}).call(this, event); if (r === false) event.preventDefault(); });\\n';
    }
    var s = document.createElement('script');
    s.nonce = N;
    s.textContent = '(function (el) {\\n' + src + '})(window.__acpiraOn[' + k + ']);';
    (document.head || document.documentElement).appendChild(s);
    s.remove();
  }
  function walk(node) {
    if (node.nodeType !== 1) return;
    rebind(node);
    var all = node.getElementsByTagName('*');
    for (var i = 0; i < all.length; i++) rebind(all[i]);
  }
  new MutationObserver(function (records) {
    for (var i = 0; i < records.length; i++) {
      var r = records[i];
      if (r.type === 'attributes') { if (/^on/i.test(r.attributeName)) rebind(r.target); }
      else for (var j = 0; j < r.addedNodes.length; j++) walk(r.addedNodes[j]);
    }
  }).observe(document, { childList: true, subtree: true, attributes: true });
  var last = 0;
  function report() {
    var h = Math.ceil(document.documentElement.getBoundingClientRect().height);
    if (h !== last) { last = h; parent.postMessage({ [KEY]: 'height', height: h }, '*'); }
  }
  function theme(t) {
    var root = document.documentElement;
    for (var name in t.vars) root.style.setProperty('--' + name, t.vars[name]);
    root.classList.toggle('dark', !!t.dark);
    root.style.colorScheme = t.dark ? 'dark' : 'light';
  }
  // The page drawn the way the reply screenshot draws the transcript: a clone with its live state (form values, canvas
  // pixels) and every stylesheet, as an SVG foreignObject on a canvas. Nested images are decoded first and the canvas is
  // drawn twice, since WebKit paints images it has not decoded yet blank on the first draw
  var lastSnapshot;
  function snapshot(id, dpr) {
    function send(png) { parent.postMessage({ [KEY]: 'snapshot', id: id, png: png }, '*'); }
    try {
      var de = document.documentElement, w = Math.max(1, de.clientWidth), h = Math.max(1, Math.ceil(de.getBoundingClientRect().height));
      var clone = de.cloneNode(true), sel = 'input, textarea, select, canvas';
      var from = de.querySelectorAll(sel), to = clone.querySelectorAll(sel), i, j;
      for (i = 0; i < from.length; i++) {
        var a = from[i], b = to[i];
        if (!b) continue;
        if (a.tagName === 'CANVAS') {
          try {
            var im = document.createElement('img');
            im.src = a.toDataURL('image/png');
            im.className = a.className;
            im.style.cssText = a.style.cssText + ';width:' + a.clientWidth + 'px;height:' + a.clientHeight + 'px';
            b.parentNode.replaceChild(im, b);
          } catch (err) {}
        } else if (a.tagName === 'TEXTAREA') b.textContent = a.value;
        else if (a.tagName === 'SELECT') { for (j = 0; j < a.options.length; j++) b.options[j].toggleAttribute('selected', a.options[j].selected); }
        else if (a.type === 'checkbox' || a.type === 'radio') b.toggleAttribute('checked', a.checked);
        else b.setAttribute('value', a.value);
      }
      var css = '';
      for (i = 0; i < document.styleSheets.length; i++) {
        try { var rules = document.styleSheets[i].cssRules; for (j = 0; j < rules.length; j++) css += rules[j].cssText + '\\n'; } catch (err) {}
      }
      var drop = clone.querySelectorAll('script, style, link, meta');
      for (i = 0; i < drop.length; i++) drop[i].remove();
      var style = document.createElement('style');
      style.textContent = css;
      clone.insertBefore(style, clone.firstChild);
      clone.style.width = w + 'px';
      var svg = '<svg xmlns="http://www.w3.org/2000/svg" width="' + w + '" height="' + h + '"><foreignObject width="100%" height="100%">' +
        new XMLSerializer().serializeToString(clone) + '</foreignObject></svg>';
      var nested = [], imgs = clone.querySelectorAll('img');
      for (i = 0; i < imgs.length; i++) {
        var src = imgs[i].getAttribute('src') || '';
        if (src.indexOf('data:') === 0) { var p = new Image(); p.src = src; nested.push(p.decode().catch(function () {})); }
      }
      Promise.all(nested).then(function () {
        var img = new Image();
        img.onerror = function () { send(null); };
        img.onload = function () {
          var c = document.createElement('canvas'), g = c.getContext('2d');
          c.width = Math.round(w * dpr); c.height = Math.round(h * dpr);
          g.scale(dpr, dpr);
          g.drawImage(img, 0, 0, w, h);
          setTimeout(function () {
            try { g.clearRect(0, 0, w, h); g.drawImage(img, 0, 0, w, h); send(c.toDataURL('image/png')); } catch (err) { send(null); }
          }, 32);
        };
        img.src = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
      });
    } catch (err) { send(null); }
  }
  addEventListener('message', function (e) {
    if (e.source !== parent || !e.data) return;
    if (e.data[KEY] === 'theme') theme(e.data);
    else if (e.data[KEY] === 'snapshot') {
      // The page repeats the ask until it hears back: one drawing per ask, made once the page has loaded and styled
      var id = String(e.data.id), dpr = Math.min(Math.max(Number(e.data.dpr) || 1, 1), 3);
      if (id === lastSnapshot) return;
      lastSnapshot = id;
      if (document.readyState === 'complete') snapshot(id, dpr);
      else addEventListener('load', function () { snapshot(id, dpr); }, { once: true });
    }
  });
  addEventListener('click', function (e) {
    var a = e.target && e.target.closest && e.target.closest('a[href]');
    if (a && !/^#/.test(a.getAttribute('href') || '')) e.preventDefault();
  }, true);
  document.addEventListener('DOMContentLoaded', function () {
    walk(document.documentElement);
    var ro = new ResizeObserver(report);
    ro.observe(document.documentElement);
    if (document.body) ro.observe(document.body);
    report();
  });
  addEventListener('load', report);
  theme(__THEME__);
})();`;

const escapeAttr = (s: string) => s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;');
// A script body inside the document: `</script` would end it early
const scriptText = (s: string) => s.replace(/<\/(script)/gi, '<\\/$1');

const CSP_META = /<meta\b[^>]*\bhttp-equiv\s*=\s*["']?\s*content-security-policy\b[^>]*>/gi;
const BASE_TAG = /<base\b[^>]*>/gi;
const SCRIPT_TAG = /<script\b([^>]*)>/gi;
const NONCE_ATTR = /\s+nonce\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)/gi;
const SRC_ATTR = /\ssrc\s*=/i;

export interface WidgetDocOptions {
  // The page's script nonce (inherited by the frame's CSP); a fresh one where the page has no CSP (the harness)
  nonce: string;
  // Tailwind's browser build (`@tailwindcss/browser`), inlined so the frame needs no network
  tailwind: string;
  theme: WidgetTheme;
}

export function buildWidgetDoc(source: string, { nonce, tailwind, theme }: WidgetDocOptions): string {
  const n = escapeAttr(nonce);
  // The widget's own CSP would only narrow ours and could block the nonce'd scripts; a <base> would point links away
  // Inline scripts run under our nonce (a nonce of the page's own would match nothing). A script with `src` gets none: a
  // nonce lets an external script load from anywhere, so leaving it out is what keeps CDN code out of the frame
  const body = source.replace(CSP_META, '').replace(BASE_TAG, '')
    .replace(SCRIPT_TAG, (_tag, attrs: string) => {
      const own = attrs.replace(NONCE_ATTR, '');
      return SRC_ATTR.test(own) ? `<script${own}>` : `<script nonce="${n}"${own}>`;
    });
  const csp = `default-src 'none'; script-src 'nonce-${n}'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:`;
  const vars = Object.entries(theme.vars).map(([k, v]) => `--${k}: ${v};`).join(' ');
  const base = `:root { ${vars} color-scheme: ${theme.dark ? 'dark' : 'light'}; }
html, body { margin: 0; background: transparent; }
body { color: var(--foreground); font-family: var(--font-sans); font-size: var(--font-size); line-height: 1.5; overflow-wrap: anywhere; }`;
  return `<!doctype html><html class="${theme.dark ? 'dark' : ''}"><head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${csp}">
<meta name="viewport" content="width=device-width, initial-scale=1">
<script nonce="${n}">${scriptText(BOOTSTRAP.replace('__THEME__', JSON.stringify(theme)))}</script>
<style>${base}</style>
<script nonce="${n}">${scriptText(tailwind)}</script>
<style type="text/tailwindcss">${TAILWIND_THEME}</style>
</head>
${body}`;
}

// The values of WIDGET_VARS as the element sees them (custom properties compute with their var() references resolved)
export function readWidgetTheme(el: Element, dark: boolean): WidgetTheme {
  const css = getComputedStyle(el);
  const vars: Record<string, string> = {};
  for (const [name, token] of Object.entries(WIDGET_VARS)) {
    const value = css.getPropertyValue(token).trim();
    if (value) vars[name] = value;
  }
  return { vars, dark };
}

// The page's script nonce: VS Code and IntelliJ give the page's own scripts one; `.nonce` keeps it after the attribute is hidden
export function pageNonce(): string {
  const found = typeof document !== 'undefined' ? (document.querySelector('script[nonce]') as HTMLScriptElement | null)?.nonce : undefined;
  if (found) return found;
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return btoa(String.fromCharCode(...bytes)).replace(/[+/=]/g, '');
}
