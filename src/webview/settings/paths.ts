// Path display for the settings pages: pure string logic, no DOM

// A Windows path: a drive letter or a UNC share. Its separators are either slash and its case does not matter
const isWindowsPath = (p: string) => /^[a-zA-Z]:([\\/]|$)|^\\\\/.test(p);

// The separator a path is displayed with: backslash for Windows paths, slash otherwise
export const pathSep = (p: string) => (isWindowsPath(p) ? '\\' : '/');

// Filesystem path shortened for display: inside the workspace → relative, under home → ~/… (~\… on Windows).
// A Windows path is shown with backslashes only, whatever mix of separators it arrived with
export function shortPath(path: string, env: { home: string; cwd: string }): string {
  const win = isWindowsPath(path);
  const sep = win ? '\\' : '/';
  const shown = win ? path.replace(/\//g, '\\') : path;
  const key = (p: string) => (win ? p.replace(/\//g, '\\').toLowerCase() : p);
  const strip = (root: string) => {
    if (!root) return undefined;
    const r = key(root).replace(/[\\/]+$/, '');
    const p = key(shown);
    if (p !== r && !p.startsWith(`${r}${sep}`)) return undefined;
    return shown.slice(r.length).replace(/^[\\/]+/, '');
  };
  const rel = strip(env.cwd);
  if (rel !== undefined) return rel || '.';
  const home = strip(env.home);
  return home !== undefined ? (home ? `~${sep}${home}` : '~') : shown;
}
