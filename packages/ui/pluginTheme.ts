// The plugins' theme layer (HostMsg pluginTheme, docs/dev/plugins.md): one style element after the page's own sheets,
// replaced on every push and removed when the engine sends none (no theme, or safe mode)
const ID = 'acpira-plugin-theme';

export function applyPluginTheme(css: string) {
  let el = document.getElementById(ID);
  if (!css) { el?.remove(); return; }
  if (!el) { el = document.createElement('style'); el.id = ID; }
  // Moved to the end on every push, so it also follows a sheet a lazy chunk added since
  document.head.appendChild(el);
  if (el.textContent !== css) el.textContent = css;
}
