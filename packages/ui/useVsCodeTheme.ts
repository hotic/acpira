import { useEffect, useState } from 'react';
import type { Theme } from './look';

// VS Code hangs theme classes on body (vscode-dark / vscode-light / vscode-high-contrast*); follow it.
// This is the host's scheme only — the theme setting resolves against it in App (resolveTheme)
export function useVsCodeTheme(): Theme {
  const read = () => (document.body.classList.contains('vscode-light') || document.body.classList.contains('vscode-high-contrast-light') ? 'light' : 'dark');
  const [theme, setTheme] = useState<Theme>(read);
  useEffect(() => {
    const mo = new MutationObserver(() => setTheme(read()));
    mo.observe(document.body, { attributes: true, attributeFilter: ['class'] });
    return () => mo.disconnect();
  }, []);
  return theme;
}
