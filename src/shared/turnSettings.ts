import type { SessionControls, TurnSettings } from './transcript';
import { isModelControl, shapeFor, type ModelShapes } from './modelShapes';

// Persist wire IDs rather than display names, including model effort variants.
export function captureTurnSettings(controls: SessionControls): TurnSettings {
  return {
    modeId: controls.modeId,
    config: Object.fromEntries(controls.options.flatMap(c => c.value === undefined ? [] : [[c.id, c.value]])),
  };
}

// The history editor never calls set_config_option, so its dependent controls must come from the chosen model itself,
// keeping the editor's values they still offer. The learned shape comes first: it is recorded from a ready session's agent
// truth, while the live view can pair its model with another model's parameters (`previewControls` while the session
// starts, the optimistic overlay while a composer model switch is in flight). Then the live controls for the live model,
// then a same-family sibling's shape. An unknown model keeps the snapshot; the host lets the agent settle the rest.
function reshape(fallback: SessionControls, settings: TurnSettings, live: SessionControls, shapes: ModelShapes | undefined): SessionControls {
  const model = live.options.find(isModelControl);
  const value = model && settings.config[model.id];
  if (!model || !value || !model.options.some(o => o.id === value)) return fallback;
  const shape = shapes?.[value] ?? (value === model.value ? undefined : shapeFor(shapes, model, value));
  if (shape) return controlsForTurn({ ...live, options: [...live.options.filter(isModelControl), ...shape] }, settings);
  return value === model.value ? controlsForTurn(live, settings) : fallback;
}

// The editor's controls when it opens on a historical turn
export function openTurnControls(live: SessionControls, settings: TurnSettings | undefined, shapes?: ModelShapes): SessionControls {
  const fallback = controlsForTurn(live, settings);
  return settings ? reshape(fallback, settings, live, shapes) : fallback;
}

// One pick in the editor; a model pick brings that model's own parameters along
export function editTurnConfig(editor: SessionControls, live: SessionControls, id: string, value: string, shapes?: ModelShapes): SessionControls {
  const next = { ...editor, options: editor.options.map(c => c.id === id ? { ...c, value } : c) };
  const control = live.options.find(c => c.id === id);
  return control && isModelControl(control) ? reshape(next, captureTurnSettings(next), live, shapes) : next;
}

// Historical selections may disappear after a CLI update. Use current choices
// for unavailable values; never guess a replacement from a model's display name.
export function controlsForTurn(controls: SessionControls, settings?: TurnSettings): SessionControls {
  return {
    ...controls,
    modeId: controls.modes.some(m => m.id === settings?.modeId) ? settings!.modeId : controls.modeId,
    options: controls.options.map(c => ({
      ...c,
      value: c.options.some(o => o.id === settings?.config[c.id]) ? settings!.config[c.id] : c.value,
    })),
  };
}
