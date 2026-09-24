import { describe, expect, it } from 'vitest';
import { composerControls, effortOptions, familyLabel, fastOn, fastValue, isFastControl, modelConfigChip, presentReasoning, reasoningChip, reasoningVisible, thoughtCorrection } from '../src/shared/composerControls';
import { groupModels } from '../src/shared/models';
import type { ConfigControl } from '../src/shared/transcript';

const kimiThink = (value: string, names: string[]): ConfigControl => ({
  id: 'thinking', name: 'Thinking', category: 'thought_level', value,
  options: names.map(name => ({ id: name, name: `Thinking ${name[0]!.toUpperCase()}${name.slice(1)}` })),
});

describe('shared composer controls', () => {
  it('keeps native Kimi reasoning out of model-name grouping', () => {
    const thinking: ConfigControl = {
      id: 'thinking', name: 'Thinking', category: 'thought_level', value: 'high',
      options: ['Low', 'High', 'Max'].map(name => ({ id: name.toLowerCase(), name: `Thinking ${name}` })),
    };
    const model: ConfigControl = { id: 'model', name: 'Model', category: 'model', options: [{ id: 'asgard/kimi-k3', name: 'K3' }] };
    expect(composerControls([model, thinking])).toEqual({ models: [model], reasoning: [thinking], modelConfig: [], collaboration: [], other: [] });
    expect(composerControls([{ ...thinking, category: undefined }]).reasoning).toHaveLength(1);
  });

  it('places native model_config beside reasoning without parsing it into model families', () => {
    const speed: ConfigControl = { id: 'speed', name: 'Speed', category: 'model_config', value: 'standard', options: [
      { id: 'standard', name: 'Standard' }, { id: 'fast', name: 'Fast' },
    ] };
    const model: ConfigControl = { id: 'model', name: 'Model', category: 'model', value: 'claude-opus-5-medium', options: [
      { id: 'claude-opus-5-medium', name: 'Claude Opus 5' },
    ] };
    const reasoning = kimiThink('medium', ['low', 'medium', 'high', 'xhigh', 'max']);
    const custom = { ...speed, id: 'custom-speed', category: '_custom' };
    const parameter: ConfigControl = { id: 'context', name: 'Context', category: 'model_config', value: 'context:1m', options: [
      { id: 'context:normal', name: 'Context Standard' }, { id: 'context:1m', name: 'Context 1M' },
    ] };
    expect(composerControls([model, speed, reasoning, parameter, custom])).toEqual({
      models: [model], reasoning: [reasoning], modelConfig: [speed, parameter], collaboration: [], other: [custom],
    });
    expect(composerControls([speed]).modelConfig).toEqual([speed]);
    expect(isFastControl(speed)).toBe(true);
    expect(modelConfigChip(speed)).toBeUndefined();
    expect(modelConfigChip({ ...speed, value: 'fast' })).toBe('Fast');
    expect(isFastControl(parameter)).toBe(false);
    expect(modelConfigChip(parameter)).toBe('Context 1M');
    expect(isFastControl({ ...speed, options: [...speed.options, { id: 'auto', name: 'Auto' }] })).toBe(false);
    expect(isFastControl({ ...speed, options: [{ id: 's', name: 'Standard' }, { id: 'f', name: 'Fast' }] })).toBe(false);
  });

  it('every Fast shape reads as one "Fast" switch: Devin speed select, codex fast-mode and claude fast booleans', () => {
    const fast: ConfigControl = { id: 'fast-mode', name: 'Fast mode', category: 'model_config', type: 'boolean', value: 'true',
      options: [{ id: 'false', name: 'Off' }, { id: 'true', name: 'On' }] };
    const claude: ConfigControl = { ...fast, id: 'fast', name: 'Fast mode' };
    const speed: ConfigControl = { id: 'speed', name: 'Speed', category: 'model_config', value: 'fast', options: [
      { id: 'standard', name: 'Standard' }, { id: 'fast', name: 'Fast' },
    ] };
    for (const c of [fast, claude, speed]) {
      expect(isFastControl(c)).toBe(true);
      expect(fastOn(c)).toBe(true);
      expect(modelConfigChip(c)).toBe('Fast');
    }
    expect(modelConfigChip({ ...fast, value: 'false' })).toBeUndefined();
    expect(fastValue(fast, false)).toBe('false');
    expect(fastValue(speed, false)).toBe('standard');
    // "breakfast" is not Fast
    expect(isFastControl({ ...fast, id: 'breakfast', name: 'Breakfast' })).toBe(false);
  });

  it('codex collaboration_mode joins the working modes on the left, not the right-side option chips', () => {
    const collab: ConfigControl = { id: 'collaboration_mode', name: 'Collaboration mode', category: 'collaboration_mode', value: 'default', options: [
      { id: 'default', name: 'Default' }, { id: 'plan', name: 'Plan', description: 'Plan before making changes' },
    ] };
    expect(composerControls([collab])).toEqual({ models: [], reasoning: [], modelConfig: [], collaboration: [collab], other: [] });
  });

  it('a non-Fast boolean control chips as its own name only while on', () => {
    const toggle: ConfigControl = { id: 'auto-review', name: 'Auto review', category: 'model_config', type: 'boolean', value: 'true',
      options: [{ id: 'false', name: 'Off' }, { id: 'true', name: 'On' }] };
    expect(modelConfigChip(toggle)).toBe('Auto review');
    expect(modelConfigChip({ ...toggle, value: 'false' })).toBeUndefined();
    expect(isFastControl(toggle)).toBe(false);
    // The synthetic Off/On pair must never read as a model family
    expect(composerControls([toggle])).toEqual({ models: [], reasoning: [], modelConfig: [toggle], collaboration: [], other: [] });
    // Even a `model`-categorized boolean stays out of the models bucket — Off/On is never a family
    expect(composerControls([{ ...toggle, category: 'model' }])).toEqual({ models: [], reasoning: [], modelConfig: [], collaboration: [], other: [{ ...toggle, category: 'model' }] });
  });

  it('normalizes and orders Grok labels while preserving exact wire IDs', () => {
    expect(effortOptions([
      { id: 'xhigh', name: 'Extra High Effort' },
      { id: 'opaque-high', name: 'High Effort' },
      { id: 'low', name: 'Low Effort' },
      { id: 'vendor-auto', name: 'Adaptive budget' },
    ])).toEqual([
      { id: 'low', name: 'Low' },
      { id: 'opaque-high', name: 'High' },
      { id: 'xhigh', name: 'XHigh' },
      { id: 'vendor-auto', name: 'Adaptive budget' },
    ]);
  });

  it('settings thinking rows drop Effort without changing hide keys', () => {
    const control: ConfigControl = {
      id: 'reasoning_effort', name: 'Reasoning', category: 'thought_level',
      options: [
        { id: 'xhigh', name: 'Extra High Effort' },
        { id: 'high', name: 'High Effort' },
        { id: 'medium', name: 'Medium Effort' },
        { id: 'low', name: 'Low Effort' },
      ],
    };
    const families = groupModels(control.options);
    expect(families.map(f => f.name)).toEqual(['Extra High Effort', 'High Effort', 'Medium Effort', 'Low Effort']);
    expect(families.map(f => familyLabel(control, f))).toEqual(['XHigh', 'High', 'Medium', 'Low']);
  });

  it('drops a leftover thinking toggle when Kimi appends it onto K3 efforts', () => {
    const control = kimiThink('on', ['low', 'high', 'max', 'on']);
    expect(presentReasoning(control)).toMatchObject({
      value: 'high', off: false, offId: undefined,
      efforts: [{ id: 'low', name: 'Low' }, { id: 'high', name: 'High' }, { id: 'max', name: 'Max' }],
    });
    expect(thoughtCorrection(control)).toBe('high');
    expect(reasoningChip(control)).toBe('High');
    expect(reasoningVisible(control)).toBe(true);
  });

  it('drops a leftover effort when Kimi appends it onto a toggle-only model', () => {
    const control = kimiThink('high', ['on', 'high']);
    expect(presentReasoning(control)).toMatchObject({ value: 'on', efforts: [], onId: 'on', off: false });
    expect(thoughtCorrection(control)).toBe('on');
    expect(reasoningChip(control)).toBeUndefined();
    expect(reasoningVisible(control)).toBe(false);
  });

  it('keeps DeepSeek off beside Low/Medium/High/Max and does not treat off as a pill', () => {
    const control = kimiThink('off', ['off', 'low', 'medium', 'high', 'max']);
    const p = presentReasoning(control);
    expect(p.off).toBe(true);
    expect(p.offId).toBe('off');
    expect(p.efforts.map(o => o.name)).toEqual(['Low', 'Medium', 'High', 'Max']);
    expect(thoughtCorrection(control)).toBeUndefined();
    expect(reasoningChip(control)).toBeUndefined();
    expect(reasoningVisible(control)).toBe(true);
    expect(reasoningChip({ ...control, value: 'high' })).toBe('High');
  });

  it('leaves a clean K3 effort list alone', () => {
    const control = kimiThink('max', ['low', 'high', 'max']);
    expect(presentReasoning(control).value).toBe('max');
    expect(thoughtCorrection(control)).toBeUndefined();
    expect(reasoningChip(control)).toBe('Max');
  });

  it('does not rewrite a model control', () => {
    expect(thoughtCorrection({ id: 'model', name: 'Model', category: 'model', value: 'asgard/kimi-k3', options: [] })).toBeUndefined();
  });

  it('retains legacy Devin model variants and respects explicit custom categories', () => {
    const model: ConfigControl = { id: 'legacy', name: 'Model', options: [
      { id: 'penguin-medium', name: 'Penguin Medium' }, { id: 'penguin-max', name: 'Penguin Max' },
    ] };
    const custom = { ...model, id: 'custom', category: 'custom' };
    expect(composerControls([model, custom])).toEqual({ models: [model], reasoning: [], modelConfig: [], collaboration: [], other: [custom] });
  });
});
