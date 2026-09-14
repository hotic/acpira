import { createContext, useContext, type CSSProperties } from 'react';
import type { DiffMarkers, SettingsView } from '@shared/settings';

export type Theme = 'dark' | 'light';

// The scheme the shell renders in (the theme setting resolved against the host); Prose reads it for mermaid, everything else goes through data-theme
export const ThemeContext = createContext<Theme>('dark');
export const useTheme = () => useContext(ThemeContext);

// The theme setting resolved against the VS Code theme: `auto` follows the host, a fixed scheme ignores its palette (data-theme-fixed)
export function resolveTheme(setting: SettingsView['theme'], host: Theme): { theme: Theme; fixed: boolean } {
  return setting === 'auto' ? { theme: host, fixed: false } : { theme: setting, fixed: true };
}

// The user-facing rendering preferences from the settings page (as opposed to the LAB-only appearance axes): the shell root turns them into
// data attributes and the two font-size variables the type scale in tokens.css derives from. Absent in the LAB, so the defaults paint
export interface ShellLook {
  fixedTheme: boolean;
  uiFontSize: number;
  codeFontSize: number;
  diffMarkers: DiffMarkers;
  fontSmoothing: boolean;
  sessionListPosition: SettingsView['sessionListPosition'];
}

export function lookFromSettings(s: SettingsView): ShellLook {
  return { fixedTheme: s.theme !== 'auto', uiFontSize: s.uiFontSize, codeFontSize: s.codeFontSize, diffMarkers: s.diffMarkers, fontSmoothing: s.fontSmoothing, sessionListPosition: s.sessionListPosition };
}

export function lookAttrs(look?: ShellLook) {
  if (!look) return {};
  return {
    'data-theme-fixed': look.fixedTheme ? '' : undefined,
    'data-diff': look.diffMarkers,
    'data-smoothing': look.fontSmoothing ? 'antialiased' : undefined,
    style: { '--ui-font-size': `${look.uiFontSize}px`, '--code-font-size': `${look.codeFontSize}px` } as CSSProperties,
  };
}
