import { useSyncExternalStore } from 'react';
import {
  effectiveShortcut,
  formatShortcut,
  matchesShortcut,
  sanitizeKeybindings,
  type KeyEventLike,
  type KeybindingMap,
  type ShortcutId,
} from '@acpira/shared/keybindings';
import { macOS } from './desktop';

// Live keybindings mirror synced from SettingsView (App.tsx), readable inside keydown handlers without threading props
let activeKeybindings: KeybindingMap = {};
const listeners = new Set<() => void>();

export function setActiveKeybindings(next: KeybindingMap | undefined) {
  const clean = sanitizeKeybindings(next);
  if (JSON.stringify(clean) === JSON.stringify(activeKeybindings)) return;
  activeKeybindings = clean;
  for (const l of listeners) l();
}

export function getActiveKeybindings(): KeybindingMap {
  return activeKeybindings;
}

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => { listeners.delete(l); };
};

export function useActiveKeybindings(): KeybindingMap {
  return useSyncExternalStore(subscribe, getActiveKeybindings);
}

export function hitShortcut(e: KeyEventLike, id: ShortcutId, mac = macOS): boolean {
  return matchesShortcut(e, id, activeKeybindings, mac);
}

export function shortcutHint(id: ShortcutId, mac = macOS): string {
  return formatShortcut(effectiveShortcut(id, activeKeybindings), mac);
}
