import type { ConfigControl, SessionControls } from './transcript';
import { groupModels } from './models';

// Per-agent memory of the dependent controls (effort, Fast, other model parameters) each model came with, keyed by the
// model option id. Agents reshape these per model (Devin 3000.11.3: SWE-2 has no `speed` and only medium / high / max
// effort), and only a live `set_config_option` reveals the new shape; the history editor switches models locally, so it
// reads a model's parameters from here instead of keeping the previous model's.
export type ModelShapes = Record<string, ConfigControl[]>;

export function isModelControl(c: Pick<ConfigControl, 'id' | 'category'>): boolean {
  return c.category === 'model' || (!c.category && c.id === 'model');
}

// Structure only: a value change (another effort picked) is not a new shape and must not rewrite the shared prefs file
const structure = (controls: ConfigControl[]) => JSON.stringify(controls.map(c => [c.id, c.category, c.type, c.options.map(o => o.id)]));

// The map with the current model's shape recorded, or undefined when there is no model or its shape is already known
export function learnShape(shapes: ModelShapes | undefined, controls: SessionControls): ModelShapes | undefined {
  const model = controls.options.find(isModelControl);
  if (!model?.value) return;
  const shape = controls.options.filter(c => !isModelControl(c));
  const known = shapes?.[model.value];
  if (known && structure(known) === structure(shape)) return;
  return { ...shapes, [model.value]: shape };
}

// A model's remembered shape; an unseen id borrows one from its family (Devin's effort and Fusion variants share the parameters)
export function shapeFor(shapes: ModelShapes | undefined, model: ConfigControl, value: string): ConfigControl[] | undefined {
  if (!shapes) return;
  if (shapes[value]) return shapes[value];
  const family = groupModels(model.options).find(f => f.variants.some(v => v.id === value));
  const sibling = family?.variants.find(v => shapes[v.id]);
  return sibling && shapes[sibling.id];
}
