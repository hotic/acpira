// Appearance axes: one-to-one with the acpira.appearance.* settings. The order is the digit order of the combo code — do not reorder.
export interface Appearance {
  density: 'cozy' | 'compact' | 'airy';
  radius: '12' | '8' | '16';
  surface: 'hairline' | 'tonal' | 'stroke';
  font: 'system' | 'inter' | 'geist';
  userMessage: 'bubble' | 'block' | 'plain';
  toolLine: 'text' | 'icon' | 'rich';
  thought: 'text' | 'shimmer' | 'orb';
  sessions: 'dropdown' | 'drawer';
  composer: 'island' | 'flush';
  beam: 'line' | 'none' | 'pulse' | 'full';
  beamColor: 'mono' | 'ocean' | 'colorful';
  send: 'accent' | 'icon' | 'metal';
  accent: 'brand' | 'agent' | 'vscode';
  // One switch: `none` kills transitions, entrances and the shimmer; the Orb (a canvas) keeps turning either way
  motion: 'on' | 'none';
  // Codex keeps one manual process fold per turn: current activity while running, elapsed time when done;
  // cursor only folds runs of read-only actions, edits and commands stay visible
  fold: 'codex' | 'cursor';
}

export type AxisKey = keyof Appearance;

export interface AxisDef<K extends AxisKey = AxisKey> {
  key: K;
  label: string;
  group: 'Structure' | 'Conversation' | 'Composer' | 'Ambience';
  options: { value: Appearance[K]; label: string }[];
}

// Axis labels are English-source and LAB-only; the production settings page exposes just `motion` (with its own i18n labels), the rest are design decisions
export const AXES: AxisDef[] = [
  { key: 'density', label: 'Density', group: 'Structure', options: [{ value: 'cozy', label: 'Cozy' }, { value: 'compact', label: 'Compact' }, { value: 'airy', label: 'Airy' }] },
  { key: 'radius', label: 'Radius', group: 'Structure', options: [{ value: '12', label: '12' }, { value: '8', label: '8' }, { value: '16', label: '16' }] },
  { key: 'surface', label: 'Surface', group: 'Structure', options: [{ value: 'hairline', label: 'hairline' }, { value: 'tonal', label: 'Tonal' }, { value: 'stroke', label: 'Stroke' }] },
  { key: 'font', label: 'Font', group: 'Structure', options: [{ value: 'system', label: 'System' }, { value: 'inter', label: 'Inter' }, { value: 'geist', label: 'Geist' }] },
  { key: 'userMessage', label: 'User message', group: 'Conversation', options: [{ value: 'bubble', label: 'Right bubble' }, { value: 'block', label: 'Block' }, { value: 'plain', label: 'Plain' }] },
  { key: 'toolLine', label: 'Tool line', group: 'Conversation', options: [{ value: 'text', label: 'Text' }, { value: 'icon', label: 'Icon' }, { value: 'rich', label: 'Icon + meta' }] },
  { key: 'thought', label: 'Thinking', group: 'Conversation', options: [{ value: 'text', label: 'Text' }, { value: 'shimmer', label: 'Shimmer' }, { value: 'orb', label: 'Orb' }] },
  { key: 'sessions', label: 'Sessions', group: 'Conversation', options: [{ value: 'dropdown', label: 'Dropdown' }, { value: 'drawer', label: 'Drawer' }] },
  { key: 'composer', label: 'Composer', group: 'Composer', options: [{ value: 'island', label: 'Island' }, { value: 'flush', label: 'Flush' }] },
  { key: 'beam', label: 'Beam', group: 'Composer', options: [{ value: 'line', label: 'Line' }, { value: 'none', label: 'None' }, { value: 'pulse', label: 'Pulse' }, { value: 'full', label: 'Full border' }] },
  { key: 'beamColor', label: 'Beam color', group: 'Composer', options: [{ value: 'mono', label: 'mono' }, { value: 'ocean', label: 'ocean' }, { value: 'colorful', label: 'colorful' }] },
  { key: 'send', label: 'Send button', group: 'Composer', options: [{ value: 'accent', label: 'Accent' }, { value: 'icon', label: 'Icon only' }, { value: 'metal', label: 'MetalFx' }] },
  { key: 'accent', label: 'Accent', group: 'Ambience', options: [{ value: 'brand', label: 'Amber' }, { value: 'agent', label: 'Per-agent' }, { value: 'vscode', label: 'Follow VS Code' }] },
  { key: 'motion', label: 'Motion', group: 'Ambience', options: [{ value: 'on', label: 'On' }, { value: 'none', label: 'None' }] },
  { key: 'fold', label: 'Folding', group: 'Conversation', options: [{ value: 'codex', label: 'Whole process' }, { value: 'cursor', label: 'Read-only only' }] },
];

// Combo code ↔ Appearance: one option index digit per axis
export function encodeAppearance(a: Appearance): string {
  return AXES.map(ax => ax.options.findIndex(o => o.value === a[ax.key])).join('');
}
export function decodeAppearance(code: string, fallback: Appearance): Appearance {
  if (code.length !== AXES.length || !/^\d+$/.test(code)) return fallback;
  const out = { ...fallback } as Record<AxisKey, string>;
  AXES.forEach((ax, i) => {
    const opt = ax.options[Number(code[i])];
    if (opt) out[ax.key] = opt.value;
  });
  return out as unknown as Appearance;
}

// Baseline: finalized as 201110200302100
export const BASE_APPEARANCE: Appearance = decodeAppearance('201110200302100', {
  density: 'airy', radius: '12', surface: 'tonal', font: 'inter',
  userMessage: 'block', toolLine: 'icon', thought: 'orb', sessions: 'dropdown',
  composer: 'island', beam: 'full', beamColor: 'mono', send: 'metal',
  accent: 'agent', motion: 'on', fold: 'codex',
});

// Builds an Appearance from a bag of setting values (acpira.appearance.<axis>); invalid values fall back to the baseline
export function appearanceFromSettings(get: (key: AxisKey) => unknown): Appearance {
  const out = { ...BASE_APPEARANCE } as Record<AxisKey, string>;
  for (const ax of AXES) {
    const v = get(ax.key);
    if (typeof v === 'string' && ax.options.some(o => o.value === v)) out[ax.key] = v;
  }
  return out as unknown as Appearance;
}
