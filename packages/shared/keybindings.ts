// Customizable keyboard shortcuts shared by the webview, settings sanitation and the desktop menu bar.
// Accelerators use Tauri/muda's canonical spelling (`CmdOrCtrl+Shift+B`, `Ctrl+\``) so the desktop shell can hand them
// straight to native menu items on macOS and format them for the Windows/Linux in-page menu.

export type ShortcutGroup = 'general' | 'chat' | 'sessions' | 'panel';

export type ShortcutId =
  | 'openSettings'
  | 'openShortcuts'
  | 'openFolder'
  | 'quickOpen'
  | 'toggleSidebar'
  | 'back'
  | 'forward'
  | 'closeTab'
  | 'focusComposer'
  | 'cycleMode'
  | 'openModelPicker'
  | 'invertSteerSend'
  | 'findInChat'
  | 'findNext'
  | 'findPrev'
  | 'newSession'
  | 'searchSessions'
  | 'nextSession'
  | 'prevSession'
  | 'nextAttention'
  | 'toggleToolPanel'
  | 'toggleTerminal'
  | 'openChanges'
  | 'toggleBrowser';

// Missing key: default shortcut; empty string "": explicitly unbound ("Not set"); non-empty: custom accelerator
export type KeybindingMap = Partial<Record<ShortcutId, string>>;

export interface ShortcutDef {
  id: ShortcutId;
  group: ShortcutGroup;
  defaultKey: string;
  secondaryMacKey?: string;
  secondaryOtherKey?: string;
  desktopOnly?: boolean;
}

export const SHORTCUT_DEFS: readonly ShortcutDef[] = [
  // General
  { id: 'openSettings', group: 'general', defaultKey: 'CmdOrCtrl+,', desktopOnly: true },
  { id: 'openShortcuts', group: 'general', defaultKey: 'CmdOrCtrl+/' },
  { id: 'openFolder', group: 'general', defaultKey: 'CmdOrCtrl+O', desktopOnly: true },
  { id: 'quickOpen', group: 'general', defaultKey: 'CmdOrCtrl+P', desktopOnly: true },
  { id: 'toggleSidebar', group: 'general', defaultKey: 'CmdOrCtrl+B', desktopOnly: true },
  { id: 'back', group: 'general', defaultKey: 'CmdOrCtrl+[', desktopOnly: true },
  { id: 'forward', group: 'general', defaultKey: 'CmdOrCtrl+]', desktopOnly: true },
  { id: 'closeTab', group: 'general', defaultKey: 'CmdOrCtrl+W', desktopOnly: true },
  // Chat
  { id: 'focusComposer', group: 'chat', defaultKey: 'Ctrl+L' },
  { id: 'cycleMode', group: 'chat', defaultKey: 'Shift+Tab' },
  { id: 'openModelPicker', group: 'chat', defaultKey: 'CmdOrCtrl+Shift+M' },
  { id: 'invertSteerSend', group: 'chat', defaultKey: 'CmdOrCtrl+Enter' },
  { id: 'findInChat', group: 'chat', defaultKey: 'CmdOrCtrl+F' },
  { id: 'findNext', group: 'chat', defaultKey: 'CmdOrCtrl+G', secondaryOtherKey: 'F3' },
  { id: 'findPrev', group: 'chat', defaultKey: 'CmdOrCtrl+Shift+G', secondaryOtherKey: 'Shift+F3' },
  // Sessions
  { id: 'newSession', group: 'sessions', defaultKey: 'CmdOrCtrl+N', desktopOnly: true },
  { id: 'searchSessions', group: 'sessions', defaultKey: 'CmdOrCtrl+K' },
  { id: 'nextSession', group: 'sessions', defaultKey: 'Ctrl+Tab', secondaryMacKey: 'CmdOrCtrl+Shift+]' },
  { id: 'prevSession', group: 'sessions', defaultKey: 'Ctrl+Shift+Tab', secondaryMacKey: 'CmdOrCtrl+Shift+[' },
  { id: 'nextAttention', group: 'sessions', defaultKey: 'Alt+CmdOrCtrl+A' },
  // Tool panel (desktop)
  { id: 'toggleToolPanel', group: 'panel', defaultKey: 'Alt+CmdOrCtrl+B', desktopOnly: true },
  { id: 'toggleTerminal', group: 'panel', defaultKey: 'Ctrl+`', desktopOnly: true },
  { id: 'openChanges', group: 'panel', defaultKey: 'CmdOrCtrl+Shift+D', desktopOnly: true },
  { id: 'toggleBrowser', group: 'panel', defaultKey: 'CmdOrCtrl+Shift+B', desktopOnly: true },
];

export const SHORTCUT_IDS: readonly ShortcutId[] = SHORTCUT_DEFS.map(d => d.id);

const DEF_BY_ID = new Map<ShortcutId, ShortcutDef>(SHORTCUT_DEFS.map(d => [d.id, d]));

const PUNCTUATION_KEYS = new Set(['`', '-', '=', '[', ']', '\\', ';', "'", ',', '.', '/']);
const NAMED_KEYS = new Map<string, string>([
  ['enter', 'Enter'],
  ['tab', 'Tab'],
  ['space', 'Space'],
  ['backspace', 'Backspace'],
  ['delete', 'Delete'],
  ['escape', 'Escape'],
  ['esc', 'Escape'],
  ['up', 'Up'],
  ['arrowup', 'Up'],
  ['down', 'Down'],
  ['arrowdown', 'Down'],
  ['left', 'Left'],
  ['arrowleft', 'Left'],
  ['right', 'Right'],
  ['arrowright', 'Right'],
]);
const FUNCTION_KEYS = new Set(Array.from({ length: 12 }, (_, i) => `F${i + 1}`));

function canonicalKey(raw: string): string | undefined {
  const t = raw.trim();
  if (!t) return undefined;
  if (t.length === 1) {
    if (/^[a-z]$/i.test(t)) return t.toUpperCase();
    if (/^[0-9]$/.test(t) || PUNCTUATION_KEYS.has(t)) return t;
    return undefined;
  }
  const upper = t.toUpperCase();
  if (FUNCTION_KEYS.has(upper)) return upper;
  return NAMED_KEYS.get(t.toLowerCase());
}

// Validates and normalizes an accelerator string; returns undefined for malformed or unsafe (plain typing) bindings
export function normalizeAccelerator(raw: string): string | undefined {
  const trimmed = raw.trim();
  if (!trimmed) return undefined;
  const parts = trimmed.split('+').map(p => p.trim()).filter(Boolean);
  const keyPart = parts.pop();
  if (!keyPart) return undefined;
  const key = canonicalKey(keyPart);
  if (!key) return undefined;

  let ctrl = false;
  let alt = false;
  let shift = false;
  let cmdOrCtrl = false;
  for (const mod of parts) {
    const m = mod.toLowerCase();
    if (m === 'ctrl' || m === 'control') ctrl = true;
    else if (m === 'alt' || m === 'option') alt = true;
    else if (m === 'shift') shift = true;
    else if (m === 'cmdorctrl' || m === 'commandorcontrol' || m === 'cmd' || m === 'command' || m === 'meta') cmdOrCtrl = true;
    else return undefined;
  }

  // Plain keys (or Shift+letter/symbol) belong to text input; only function keys or Shift+Tab/Enter may omit primary modifiers
  const hasPrimaryMod = cmdOrCtrl || ctrl || alt;
  const allowedShiftOnly = shift && (key === 'Tab' || key === 'Enter' || FUNCTION_KEYS.has(key));
  if (!hasPrimaryMod && !allowedShiftOnly && !FUNCTION_KEYS.has(key)) return undefined;

  const out: string[] = [];
  if (ctrl) out.push('Ctrl');
  if (alt) out.push('Alt');
  if (shift) out.push('Shift');
  if (cmdOrCtrl) out.push('CmdOrCtrl');
  out.push(key);
  return out.join('+');
}

export function sanitizeKeybindings(value: unknown): KeybindingMap {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
  const out: KeybindingMap = {};
  for (const [id, raw] of Object.entries(value as Record<string, unknown>)) {
    const def = DEF_BY_ID.get(id as ShortcutId);
    if (!def || typeof raw !== 'string') continue;
    const trimmed = raw.trim();
    if (trimmed === '') {
      out[def.id] = '';
      continue;
    }
    const norm = normalizeAccelerator(trimmed);
    if (norm && norm !== def.defaultKey) out[def.id] = norm;
  }
  return out;
}

export function effectiveShortcut(id: ShortcutId, map?: KeybindingMap): string {
  const override = map?.[id];
  if (override !== undefined) return override;
  return DEF_BY_ID.get(id)?.defaultKey ?? '';
}

export interface KeyEventLike {
  key: string;
  code: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
}

const CODE_TO_KEY: Record<string, string> = {
  Backquote: '`',
  Minus: '-',
  Equal: '=',
  BracketLeft: '[',
  BracketRight: ']',
  Backslash: '\\',
  Semicolon: ';',
  Quote: "'",
  Comma: ',',
  Period: '.',
  Slash: '/',
  Space: 'Space',
  Enter: 'Enter',
  NumpadEnter: 'Enter',
  Tab: 'Tab',
  Backspace: 'Backspace',
  Delete: 'Delete',
  Escape: 'Escape',
  ArrowUp: 'Up',
  ArrowDown: 'Down',
  ArrowLeft: 'Left',
  ArrowRight: 'Right',
};

function keyFromEvent(e: KeyEventLike): string | undefined {
  if (/^Key[A-Z]$/.test(e.code)) return e.code.slice(3);
  if (/^Digit[0-9]$/.test(e.code)) return e.code.slice(5);
  if (/^F([1-9]|1[0-2])$/.test(e.code)) return e.code;
  if (CODE_TO_KEY[e.code]) return CODE_TO_KEY[e.code];
  return canonicalKey(e.key);
}

export function matchesAccelerator(e: KeyEventLike, accel: string, mac: boolean): boolean {
  const norm = normalizeAccelerator(accel);
  if (!norm) return false;
  const parts = norm.split('+');
  const key = parts.pop()!;
  const wantCtrl = parts.includes('Ctrl');
  const wantAlt = parts.includes('Alt');
  const wantShift = parts.includes('Shift');
  const wantCmdOrCtrl = parts.includes('CmdOrCtrl');

  if (mac) {
    if (e.metaKey !== wantCmdOrCtrl) return false;
    if (e.ctrlKey !== wantCtrl) return false;
  } else {
    if (e.metaKey) return false;
    if (e.ctrlKey !== (wantCmdOrCtrl || wantCtrl)) return false;
  }
  if (e.altKey !== wantAlt || e.shiftKey !== wantShift) return false;
  return keyFromEvent(e) === key;
}

export function matchesShortcut(e: KeyEventLike, id: ShortcutId, map: KeybindingMap | undefined, mac: boolean): boolean {
  const active = effectiveShortcut(id, map);
  if (!active) return false;
  if (matchesAccelerator(e, active, mac)) return true;
  if (map?.[id] !== undefined) return false;
  const def = DEF_BY_ID.get(id);
  const secondary = mac ? def?.secondaryMacKey : def?.secondaryOtherKey;
  return secondary ? matchesAccelerator(e, secondary, mac) : false;
}

const MODIFIER_KEYS = new Set(['Meta', 'Control', 'Shift', 'Alt']);

// Captures a keydown inside the shortcut recorder: 'cancel' on Esc, 'clear' on Backspace/Delete, or a canonical accelerator
export function recordShortcutEvent(e: KeyEventLike, mac: boolean): 'cancel' | 'clear' | string | undefined {
  if (MODIFIER_KEYS.has(e.key)) return undefined;
  const noMods = !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey;
  if (noMods && e.key === 'Escape') return 'cancel';
  if (noMods && (e.key === 'Backspace' || e.key === 'Delete')) return 'clear';
  const key = keyFromEvent(e);
  if (!key) return undefined;
  const parts: string[] = [];
  if (mac) {
    if (e.ctrlKey) parts.push('Ctrl');
    if (e.altKey) parts.push('Alt');
    if (e.shiftKey) parts.push('Shift');
    if (e.metaKey) parts.push('CmdOrCtrl');
  } else {
    if (e.altKey) parts.push('Alt');
    if (e.shiftKey) parts.push('Shift');
    if (e.ctrlKey) parts.push('CmdOrCtrl');
  }
  parts.push(key);
  return normalizeAccelerator(parts.join('+'));
}

const MAC_KEY_GLYPHS: Record<string, string> = {
  Enter: '↵',
  Tab: '⇥',
  Space: '␣',
  Backspace: '⌫',
  Delete: '⌦',
  Escape: '⎋',
  Up: '↑',
  Down: '↓',
  Left: '←',
  Right: '→',
};

const OTHER_KEY_LABELS: Record<string, string> = {
  Escape: 'Esc',
  Delete: 'Del',
};

export function formatShortcut(accel: string, mac: boolean): string {
  const norm = normalizeAccelerator(accel);
  if (!norm) return '';
  const parts = norm.split('+');
  const key = parts.pop()!;
  const ctrl = parts.includes('Ctrl');
  const alt = parts.includes('Alt');
  const shift = parts.includes('Shift');
  const cmdOrCtrl = parts.includes('CmdOrCtrl');
  if (mac) {
    const mods = `${ctrl ? '⌃' : ''}${alt ? '⌥' : ''}${shift ? '⇧' : ''}${cmdOrCtrl ? '⌘' : ''}`;
    return `${mods}${MAC_KEY_GLYPHS[key] ?? key}`;
  }
  const out: string[] = [];
  if (cmdOrCtrl || ctrl) out.push('Ctrl');
  if (alt) out.push('Alt');
  if (shift) out.push('Shift');
  out.push(OTHER_KEY_LABELS[key] ?? key);
  return out.join('+');
}
