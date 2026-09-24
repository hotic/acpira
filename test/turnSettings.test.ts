import { describe, expect, it } from 'vitest';
import { captureTurnSettings, controlsForTurn, editTurnConfig, openTurnControls } from '../src/shared/turnSettings';
import { learnShape } from '../src/shared/modelShapes';
import type { SessionControls } from '../src/shared/transcript';

const controls: SessionControls = {
  modes: [{ id: 'default', name: 'Agent' }, { id: 'plan', name: 'Plan' }], modeId: 'default',
  options: [{ id: 'model', category: 'model', name: 'Model', value: 'official/k3', options: [
    { id: 'official/k3', name: 'K3' }, { id: 'custom/k3', name: 'K3' },
  ] }, { id: 'effort', name: 'Effort', value: 'high', options: [{ id: 'low', name: 'Low' }, { id: 'high', name: 'High' }] }],
};

describe('historical turn selections', () => {
  it('keeps source IDs distinct and edits settings without changing the live controls', () => {
    const saved = { modeId: 'plan', config: { model: 'custom/k3', effort: 'low' } };
    const editor = controlsForTurn(controls, saved);
    expect(captureTurnSettings(editor)).toEqual(saved);
    editor.options[0]!.value = 'official/k3';
    expect(saved.config.model).toBe('custom/k3');
    expect(controls.modeId).toBe('default');
    expect(controls.options[1]!.value).toBe('high');
  });

  it('a model pick in the editor re-reads the live shape only for the live model', () => {
    const option = (id: string) => ({ id, name: id });
    // Devin 3000.11.3: SWE-2 has no speed control and only medium / high / max effort; GPT-6 Luna has both
    const live: SessionControls = { modes: [], options: [
      { id: 'model', category: 'model', name: 'Model', value: 'swe-2-high', options: ['gpt-6-luna-medium', 'swe-2-high'].map(option) },
      { id: 'thought_level', category: 'thought_level', name: 'Effort', value: 'high', options: ['medium', 'high', 'max'].map(option) },
    ] };
    const luna: SessionControls = { modes: [], options: [
      { ...live.options[0]!, value: 'gpt-6-luna-medium' },
      { ...live.options[1]!, value: 'low', options: ['low', 'medium', 'high', 'xhigh', 'max'].map(option) },
      { id: 'speed', category: 'model_config', name: 'Speed', value: 'fast', options: ['standard', 'fast'].map(option) },
    ] };
    expect(editTurnConfig(luna, live, 'model', 'swe-2-high')).toEqual(live);
    const max = editTurnConfig({ ...luna, options: luna.options.map(c => c.id === 'thought_level' ? { ...c, value: 'max' } : c) }, live, 'model', 'swe-2-high');
    expect(max.options.find(c => c.id === 'thought_level')?.value).toBe('max');
    // A model with no remembered shape keeps the snapshot; the host settles what it no longer offers
    const other = editTurnConfig(luna, luna, 'model', 'swe-2-high');
    expect(captureTurnSettings(other).config).toEqual({ model: 'swe-2-high', thought_level: 'low', speed: 'fast' });
    expect(editTurnConfig(luna, live, 'speed', 'standard').options.find(c => c.id === 'speed')?.value).toBe('standard');
    // With SWE-2's shape remembered, switching away from the live Luna shows SWE-2's own parameters, and back again restores Luna's
    const shapes = learnShape(undefined, live);
    const swe = editTurnConfig(luna, luna, 'model', 'swe-2-high', shapes);
    expect(captureTurnSettings(swe).config).toEqual({ model: 'swe-2-high', thought_level: 'high' });
    expect(swe.options.find(c => c.id === 'thought_level')?.options.map(o => o.id)).toEqual(['medium', 'high', 'max']);
    expect(captureTurnSettings(editTurnConfig(swe, luna, 'model', 'gpt-6-luna-medium', shapes)).config)
      .toEqual({ model: 'gpt-6-luna-medium', thought_level: 'high', speed: 'fast' });
  });

  // The live view can pair a model with another model's parameters: `previewControls` while the session starts (remembered
  // model over the stored record's list) and the optimistic overlay while a composer model switch is in flight
  it('prefers the learned shape over a live view whose parameters lag behind its model', () => {
    const option = (id: string) => ({ id, name: id });
    const model = { id: 'model', category: 'model', name: 'Model', options: ['gpt-6-luna-medium', 'swe-2-high'].map(option) };
    const sweEffort = { id: 'thought_level', category: 'thought_level', name: 'Effort', value: 'high', options: ['medium', 'high', 'max'].map(option) };
    const luna: SessionControls = { modes: [], options: [
      { ...model, value: 'gpt-6-luna-medium' },
      { ...sweEffort, options: ['none', 'low', 'medium', 'high', 'xhigh', 'max'].map(option) },
      { id: 'speed', category: 'model_config', name: 'Speed', value: 'standard', options: ['standard', 'fast'].map(option) },
    ] };
    const swe: SessionControls = { modes: [], options: [{ ...model, value: 'swe-2-high' }, sweEffort] };
    const shapes = learnShape(learnShape(undefined, luna), swe);
    const lagging: SessionControls = { modes: [], options: [{ ...model, value: 'gpt-6-luna-medium' }, sweEffort] };
    const editor = openTurnControls(lagging, { config: { model: 'swe-2-high', thought_level: 'high' } }, shapes);
    const picked = editTurnConfig(editor, lagging, 'model', 'gpt-6-luna-medium', shapes);
    expect(picked.options.map(c => `${c.id}:${c.options.length}`)).toEqual(['model:2', 'thought_level:6', 'speed:2']);
    expect(captureTurnSettings(picked).config).toEqual({ model: 'gpt-6-luna-medium', thought_level: 'high', speed: 'standard' });
  });

  it('opens a historical turn with its own model\'s parameters, not the live model\'s', () => {
    const option = (id: string) => ({ id, name: id });
    const model = { id: 'model', category: 'model', name: 'Model', options: ['gpt-6-luna-medium', 'swe-2-high'].map(option) };
    const luna: SessionControls = { modes: [], options: [
      { ...model, value: 'gpt-6-luna-medium' },
      { id: 'thought_level', category: 'thought_level', name: 'Effort', value: 'low', options: ['low', 'high', 'max'].map(option) },
      { id: 'speed', category: 'model_config', name: 'Speed', value: 'standard', options: ['standard', 'fast'].map(option) },
    ] };
    const swe: SessionControls = { modes: [], options: [
      { ...model, value: 'swe-2-high' },
      { id: 'thought_level', category: 'thought_level', name: 'Effort', value: 'high', options: ['high', 'max'].map(option) },
    ] };
    const shapes = learnShape(learnShape(undefined, luna), swe);
    const turn = { config: { model: 'gpt-6-luna-medium', thought_level: 'low', speed: 'fast' } };
    // Live on SWE-2, editing a Luna Fast turn: Fast and the low effort come back with Luna's shape
    expect(captureTurnSettings(openTurnControls(swe, turn, shapes)).config).toEqual(turn.config);
    // Without the shape the live controls stand in, as before
    expect(captureTurnSettings(openTurnControls(swe, turn)).config).toEqual({ model: 'gpt-6-luna-medium', thought_level: 'high' });
  });

  it('learns a shape per model and ignores value-only changes', () => {
    const option = (id: string) => ({ id, name: id });
    const controls: SessionControls = { modes: [], options: [
      { id: 'model', category: 'model', name: 'Model', value: 'a', options: ['a', 'b'].map(option) },
      { id: 'effort', category: 'thought_level', name: 'Effort', value: 'low', options: ['low', 'high'].map(option) },
    ] };
    const shapes = learnShape(undefined, controls)!;
    expect(Object.keys(shapes)).toEqual(['a']);
    expect(learnShape(shapes, { ...controls, options: [controls.options[0]!, { ...controls.options[1]!, value: 'high' }] })).toBeUndefined();
    expect(learnShape(shapes, { ...controls, options: [controls.options[0]!, { ...controls.options[1]!, options: [option('high')] }] })?.a).toHaveLength(1);
    expect(learnShape(shapes, { modes: [], options: [controls.options[1]!] })).toBeUndefined();
  });

  it('uses live defaults for old records and unavailable historical values', () => {
    expect(controlsForTurn(controls)).toEqual(controls);
    expect(controlsForTurn(controls, { modeId: 'removed', config: { model: 'removed', effort: 'removed' } })).toEqual(controls);
  });
});
