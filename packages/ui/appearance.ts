import { createContext, useContext } from 'react';
import { BASE_APPEARANCE, type Appearance } from '@shared/appearance';

export * from '@shared/appearance';

// Structural axes are read from context by components; token-type axes go to CSS via data-* attributes
export function appearanceDataAttrs(a: Appearance) {
  return {
    'data-density': a.density,
    'data-radius': a.radius,
    'data-surface': a.surface,
    'data-font': a.font,
    'data-accent': a.accent,
    'data-motion': a.motion,
  } as const;
}

export const AppearanceContext = createContext<Appearance>(BASE_APPEARANCE);
export const useAppearance = () => useContext(AppearanceContext);
