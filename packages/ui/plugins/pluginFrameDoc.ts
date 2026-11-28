// Builds the self-contained srcdoc HTML for a plugin iframe on hosts that do not serve `/plugin-frame.html` (VS Code,
// IntelliJ, the browser harness; docs/dev/plugins.md Interface). A srcdoc frame inherits the parent page's nonce-only
// CSP, so every inline script gets the page's nonce under a stricter no-network CSP of our own, while `sandbox="allow-scripts"`
// (without allow-same-origin) keeps the frame on an opaque `null` origin isolated from the page's DOM, storage and API.

const TOKENS = [
  '--fg-1', '--fg-2', '--fg-3', '--fg-strong', '--bg-0', '--bg-1', '--bg-2', '--bg-code', '--line', '--line-strong', '--hover',
  '--active', '--focus', '--chip', '--ok', '--warn', '--danger', '--brand', '--font', '--font-mono', '--ui-font-size',
  '--code-font-size', '--row', '--r-sm', '--r-md', '--r-lg', '--gap', '--pad',
  // What the native-look kit (chrome/pluginKit.ts) builds on
  '--chip-hover', '--ctl', '--ctl-sm', '--input-bg', '--text-2', '--text-3', '--conversation-line',
];

// The tokens a plugin's document gets on its root, read where the frame sits, so it follows the page's theme, font and
// density without seeing the page
export function tokensOf(el: Element): string {
  const style = getComputedStyle(el);
  // The nearest scheme the page states (a skin's base among them), the host's only without one
  const stated = el.closest('[data-theme]')?.getAttribute('data-theme');
  const light = stated ? stated === 'light' : document.body.classList.contains('vscode-light');
  return [`color-scheme: ${light ? 'light' : 'dark'}`, ...TOKENS.map(name => [name, style.getPropertyValue(name).trim()] as const)
    .filter(([, v]) => v).map(([name, v]) => `${name}: ${v}`)].join('; ');
}

// A short name of a document, so a new version makes a new frame without a 256 KB key
export function digest(text: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < text.length; i++) h = Math.imul(h ^ text.charCodeAt(i), 0x01000193);
  return `${text.length}-${(h >>> 0).toString(36)}`;
}

// The page's script nonce: VS Code and IntelliJ stamp the page's scripts with one; `.nonce` keeps it after the attribute is hidden
export function pluginPageNonce(): string {
  const found = typeof document !== 'undefined' ? (document.querySelector('script[nonce]') as HTMLScriptElement | null)?.nonce : undefined;
  if (found) return found;
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return btoa(String.fromCharCode(...bytes)).replace(/[+/=]/g, '');
}

const escapeAttr = (s: string) => s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;');
const scriptText = (s: string) => s.replace(/<\/(script)/gi, '<\\/$1').replace(/<!--/g, '<\\!--').replace(/\u2028/g, '\\u2028').replace(/\u2029/g, '\\u2029');
const styleText = (s: string) => s.replace(/<\/(style)/gi, '<\\/$1');
const themeCss = (css: string) => `:root{${css}}html,body{margin:0;background:transparent;color:var(--fg-1);font-family:var(--font);font-size:var(--ui-font-size)}`;

// Single pass over the plugin's HTML so `<script>` bodies are copied verbatim and never scanned for tag-like strings
const LEADING_DOCTYPE = /^\s*<!doctype[^>]*>/i;
const HTML_TOKEN = /(<script\b([^>]*)>)([\s\S]*?<\/script\s*>)|<meta\b[^>]*\bhttp-equiv\s*=\s*["']?\s*content-security-policy\b[^>]*>|<base\b[^>]*>/gi;
const NONCE_ATTR = /\s+nonce\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+)/gi;
const SRC_ATTR = /\ssrc\s*=/i;

// Runs first in the frame: sets up the `window.acpira` bridge (matching desktop's PLUGIN_FRAME_HTML), listens for live
// theme / state / session / rpc / post updates from parent, and rebinds inline `on*` attributes under the frame nonce so
// plain-HTML plugins work under strict nonce-only CSPs
const BOOTSTRAP = `(function () {
  var N = document.currentScript.nonce, init = __INIT__;
  var waits = {}, hearers = [], watchers = [], readers = [], next = 0, els = window.__acpiraOn = [];
  function style(css) {
    var el = document.getElementById('acpira-theme');
    if (!el) { el = document.createElement('style'); el.id = 'acpira-theme'; document.head.insertBefore(el, document.head.firstChild); }
    el.textContent = ':root{' + (css || '') + '}html,body{margin:0;background:transparent;color:var(--fg-1);font-family:var(--font);font-size:var(--ui-font-size)}';
  }
  function tell(list, value) {
    list.slice().forEach(function (fn) { try { fn(value); } catch (err) { console.error(err); } });
  }
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
      else { for (var j = 0; j < r.addedNodes.length; j++) walk(r.addedNodes[j]); }
    }
  }).observe(document, { childList: true, subtree: true, attributes: true });
  document.addEventListener('DOMContentLoaded', function () { walk(document.documentElement); });
  function onMessage(e) {
    if (e.source !== parent) return;
    var m = e.data || {};
    if (m.acpira === 'theme') {
      style(m.css);
    } else if (m.acpira === 'result') {
      var w = waits[m.id];
      if (!w) return;
      delete waits[m.id];
      if (m.error !== undefined) w.reject(new Error(m.error)); else w.resolve(m.result);
    } else if (m.acpira === 'post') {
      tell(hearers, m.message);
    } else if (m.acpira === 'state') {
      window.acpira.state = m.state || {};
      tell(watchers, window.acpira.state);
    } else if (m.acpira === 'session') {
      window.acpira.session = m.session || null;
      tell(readers, window.acpira.session);
    }
  }
  window.acpira = {
    call: function (method, params) {
      return new Promise(function (resolve, reject) {
        var id = ++next;
        waits[id] = { resolve: resolve, reject: reject };
        parent.postMessage({ acpira: 'call', id: id, method: method, params: params }, '*');
      });
    },
    on: function (fn) {
      hearers.push(fn);
      return function () { hearers = hearers.filter(function (h) { return h !== fn; }); };
    },
    onState: function (fn) {
      watchers.push(fn);
      if (window.acpira.state) fn(window.acpira.state);
      return function () { watchers = watchers.filter(function (h) { return h !== fn; }); };
    },
    onSession: function (fn) {
      readers.push(fn);
      if (window.acpira.session !== undefined) fn(window.acpira.session);
      return function () { readers = readers.filter(function (h) { return h !== fn; }); };
    },
    scrollTo: function (turn) {
      parent.postMessage({ acpira: 'scrollTo', turn: turn }, '*');
    },
    options: init.options || {}
  };
  if ('state' in init) window.acpira.state = init.state;
  if ('session' in init) window.acpira.session = init.session;
  window.addEventListener('message', onMessage);
  parent.postMessage({ acpira: 'ready' }, '*');
})();`;

export interface PluginFrameDocOptions {
  nonce: string;
  css: string;
  kit: string;
  lib?: string;
  options?: unknown;
  state?: unknown;
  session?: unknown;
}

export function buildPluginFrameDoc(source: string, { nonce, css, kit, lib, options, state, session }: PluginFrameDocOptions): string {
  const n = escapeAttr(nonce);
  const cleaned = source.replace(LEADING_DOCTYPE, '').replace(HTML_TOKEN, (_match, open?: string, attrs?: string, tail?: string) => {
    if (!open) return '';
    const own = (attrs ?? '').replace(NONCE_ATTR, '');
    // External script tags never receive the nonce, so the CSP keeps remote code out of the frame
    return (SRC_ATTR.test(own) ? `<script${own}>` : `<script nonce="${n}"${own}>`) + (tail ?? '');
  });
  const init: Record<string, unknown> = { options: options ?? {} };
  if (state !== undefined) init.state = state;
  if (session !== undefined) init.session = session;
  const csp = `default-src 'none'; script-src 'nonce-${n}'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:`;
  const boot = scriptText(BOOTSTRAP.replace('__INIT__', () => JSON.stringify(init)));
  const runtime = lib ? `\n<script nonce="${n}">${scriptText(lib)}</script>` : '';
  return `<!doctype html><html><head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${csp}">
<style id="acpira-theme">${styleText(themeCss(css))}</style>
<style id="acpira-kit">${styleText(kit)}</style>
<script nonce="${n}">${boot}</script>${runtime}
</head>
${cleaned}`;
}
