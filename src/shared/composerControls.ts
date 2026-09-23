import type { ConfigControl, SessionOption } from './transcript';
import type { ModelFamily } from './models';
import { groupModels } from './models';

const LEVELS = ['None', 'Minimal', 'Low', 'Medium', 'High', 'XHigh', 'Max', 'Thinking'];
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

// A native Standard / Fast select shares the embedded variant's switch presentation.
export function isFastControl(control: ConfigControl): boolean {
  return control.options.length === 2 && ['standard', 'fast'].every(id => control.options.some(option => option.id === id));
}

export function modelConfigChip(control: ConfigControl): string | undefined {
  if (isFastControl(control)) return control.value === 'fast' ? 'Fast' : undefined;
  // A boolean shows its name only while on — an off toggle adds no chip clutter
  if (control.type === 'boolean') return control.value === 'true' ? control.name : undefined;
  return control.options.find(option => option.id === control.value)?.name;
}

// Settings rows reuse composer labels; hide/show keys stay on the original ACP names.
export function familyLabel(control: Pick<ConfigControl, 'id' | 'category'>, family: Pick<ModelFamily, 'name' | 'variants'>): string {
  const option = family.variants[0];
  if (option && isReasoningControl(control)) return effortLabel(option);
  return family.name;
}

// ACP capabilities choose the contents of one composer, never its layout.
// Recognize native reasoning and model parameters before applying model-name decomposition.
export function composerControls(options: ConfigControl[]) {
  const models: ConfigControl[] = [], reasoning: ConfigControl[] = [], modelConfig: ConfigControl[] = [], other: ConfigControl[] = [];
  for (const c of options) {
    if (isReasoningControl(c)) reasoning.push(c);
    else if (c.category === 'model_config') modelConfig.push(c);
    // A boolean's synthetic Off/On pair is not a model family even under a `model` category; it chips in `other`
    else if (c.type !== 'boolean' && (c.category === 'model' || (!c.category && (c.id === 'model' || groupModels(c.options).length < c.options.length)))) models.push(c);
    else other.push(c);
  }
  return { models, reasoning, modelConfig, other };
}
