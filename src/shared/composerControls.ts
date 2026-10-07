import type { ConfigControl, SessionOption } from './transcript';
import type { ModelFamily, ModelVariant } from './models';
import { groupModels } from './models';

// Ultra is the tier past Max: Codex's real reasoning_effort value ("maximum reasoning with automatic task delegation"), and
// on Claude the host's level for ultracode (xhigh + dynamic workflow orchestration), appended by the engine
const LEVELS = ['None', 'Minimal', 'Low', 'Medium', 'High', 'XHigh', 'Max', 'Ultra', 'Thinking'];
const REASONING_IDS = /^(reasoning_effort|thought_level|thinking|thinking_level)$/;

function compact(value: string): string {
  return value.toLowerCase().replace(/(?:reasoning|thinking|effort|level)/g, '').replace(/[\s_-]/g, '');
}

// Normalize presentation only; the original option IDs remain the wire values.
export function effortLabel(option: SessionOption): string {
  const canonical = (value: string) => {
    const key = compact(value);
    return key === 'extrahigh' ? 'XHigh' : LEVELS.find(level => level.toLowerCase() === key);
  };
  return canonical(option.id) ?? canonical(option.name) ?? option.name;
}

export function effortOptions(options: SessionOption[]): SessionOption[] {
  const rank = (name: string) => { const i = LEVELS.indexOf(name); return i < 0 ? LEVELS.length : i; };
  return options.map(o => ({ ...o, name: effortLabel(o) })).sort((a, b) => rank(a.name) - rank(b.name));
}

// The overdrive tier the model panel tints (the last segment of the scale, the chip's badge)
export function isUltraLevel(label: string | undefined): boolean {
  return label === 'Ultra';
}

export function isReasoningControl(control: Pick<ConfigControl, 'id' | 'category'>): boolean {
  return control.category === 'thought_level' || (!control.category && REASONING_IDS.test(control.id));
}

// Kimi folds thinking enabled/disabled into the same select as effort levels (`on` / `off`).
function toggleSide(option: SessionOption): 'on' | 'off' | undefined {
  const key = compact(option.id) || compact(option.name);
  return key === 'on' || key === 'off' ? key : undefined;
}

// Kimi appends the previous model's thinking value when the new model does not offer it:
// K3 keeps a leftover `on`, K2.7 keeps a leftover `high`. Drop the stranger, keep the native set.
function nativeThoughtOptions(options: SessionOption[]): SessionOption[] {
  const toggles = options.filter(o => toggleSide(o));
  const efforts = options.filter(o => !toggleSide(o));
  const ons = toggles.filter(o => toggleSide(o) === 'on');
  if (toggles.some(o => toggleSide(o) === 'off')) return options;
  if (ons.length && efforts.length > 1) return efforts;
  if (ons.length && efforts.length === 1) return ons;
  return options;
}

export interface ReasoningPresentation {
  efforts: SessionOption[];
  offId?: string;
  onId?: string;
  value?: string;
  off: boolean;
}

export function presentReasoning(control: ConfigControl): ReasoningPresentation {
  const native = nativeThoughtOptions(control.options);
  const efforts = effortOptions(native.filter(o => !toggleSide(o)));
  const offId = native.find(o => toggleSide(o) === 'off')?.id;
  const onId = native.find(o => toggleSide(o) === 'on')?.id;
  const value = native.some(o => o.id === control.value) ? control.value : thoughtValue(native);
  return { efforts, offId, onId, value, off: !!offId && value === offId };
}

function thoughtValue(native: SessionOption[]): string | undefined {
  const efforts = native.filter(o => !toggleSide(o));
  return efforts.find(o => o.id === 'high')?.id ?? efforts[0]?.id ?? native.find(o => toggleSide(o) === 'on')?.id ?? native[0]?.id;
}

// Host uses this after a model switch so the wire value is one the new model actually offers.
export function thoughtCorrection(control: ConfigControl): string | undefined {
  if (!isReasoningControl(control)) return;
  const native = nativeThoughtOptions(control.options);
  if (!control.value || native.some(o => o.id === control.value)) return;
  return thoughtValue(native);
}

export function reasoningVisible(control: ConfigControl): boolean {
  const p = presentReasoning(control);
  return p.efforts.length > 1 || !!p.offId;
}

export function reasoningChip(control: ConfigControl): string | undefined {
  const p = presentReasoning(control);
  if (p.off) return;
  return p.efforts.find(o => o.id === p.value)?.name;
}

// One "Fast" switch whatever the wire shape: Devin's native Standard / Fast select (`speed`), the codex / claude
// adapters' boolean (`fast-mode` "Fast mode", `fast`), and the embedded model-name variant all read the same.
export function isFastControl(control: ConfigControl): boolean {
  if (control.type === 'boolean') return /(^|[\s_-])fast($|[\s_-])/i.test(control.id) || /^fast( mode)?$/i.test(control.name);
  return control.options.length === 2 && ['standard', 'fast'].every(id => control.options.some(option => option.id === id));
}

export function fastOn(control: ConfigControl): boolean {
  return control.value === (control.type === 'boolean' ? 'true' : 'fast');
}

// The wire value that turns a Fast control on or off
export function fastValue(control: ConfigControl, on: boolean): string {
  return control.type === 'boolean' ? String(on) : on ? 'fast' : 'standard';
}

export function modelConfigChip(control: ConfigControl): string | undefined {
  if (isFastControl(control)) return fastOn(control) ? 'Fast' : undefined;
  // A boolean shows its name only while on — an off toggle adds no chip clutter
  if (control.type === 'boolean') return control.value === 'true' ? control.name : undefined;
  return control.options.find(option => option.id === control.value)?.name;
}

export interface ChipTagItem { label: string; ultra?: boolean; fast?: boolean }

// The model chip's badges, in the order they give way when the toolbar is narrow: effort (or Ultra), then Fast, then the
// rest. Provider identity stays in the expanded list; the chip reads as one model name plus its parameters. `standard` is
// the localized name of an embedded family's level-less variant
export function chipTags(cur: ModelFamily | undefined, curVar: ModelVariant | undefined, reasoning: ConfigControl[], modelConfig: ConfigControl[], standard: string): ChipTagItem[] {
  const tags: ChipTagItem[] = [];
  const fast: ChipTagItem = { label: 'Fast', fast: true };
  if (cur && curVar && (curVar.lead || cur.efforts.length > 1 || curVar.effort || curVar.fast || curVar.long)) {
    const effort = curVar.effort || (!curVar.lead && cur.efforts.length > 1 ? standard : '');
    if (effort) tags.push({ label: effort });
    if (curVar.fast) tags.push(fast);
    if (curVar.long) tags.push({ label: '1M' });
  }
  // Codex's real `ultra` effort and Claude's host-made Ultra level (ultracode) read the same: an Ultra badge
  for (const control of reasoning) {
    const level = reasoningChip(control);
    if (level) tags.push({ label: level, ultra: isUltraLevel(level) });
  }
  for (const control of modelConfig) {
    const label = modelConfigChip(control);
    if (label) tags.push(isFastControl(control) ? fast : { label });
  }
  return tags;
}

// Settings rows reuse composer labels; hide/show keys stay on the original ACP names.
export function familyLabel(control: Pick<ConfigControl, 'id' | 'category'>, family: Pick<ModelFamily, 'name' | 'variants'>): string {
  const option = family.variants[0];
  if (option && isReasoningControl(control)) return effortLabel(option);
  return family.name;
}

// ACP capabilities choose the contents of one composer, never its layout.
// Recognize native reasoning and model parameters before applying model-name decomposition.
// Codex's `collaboration_mode` (default / plan) is a working mode next to the permission modes, so it sits on the left.
export function composerControls(options: ConfigControl[]) {
  const models: ConfigControl[] = [], reasoning: ConfigControl[] = [], modelConfig: ConfigControl[] = [], collaboration: ConfigControl[] = [], other: ConfigControl[] = [];
  for (const c of options) {
    if (isReasoningControl(c)) reasoning.push(c);
    else if (c.category === 'collaboration_mode' && c.type !== 'boolean') collaboration.push(c);
    else if (c.category === 'model_config') modelConfig.push(c);
    // A boolean's synthetic Off/On pair is not a model family even under a `model` category; it chips in `other`
    else if (c.type !== 'boolean' && (c.category === 'model' || (!c.category && (c.id === 'model' || groupModels(c.options).length < c.options.length)))) models.push(c);
    else other.push(c);
  }
  return { models, reasoning, modelConfig, collaboration, other };
}
