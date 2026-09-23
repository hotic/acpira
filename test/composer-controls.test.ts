import { describe, expect, it } from 'vitest';
import { composerControls, effortOptions, familyLabel, isFastControl, modelConfigChip, presentReasoning, reasoningChip, reasoningVisible, thoughtCorrection } from '../src/shared/composerControls';
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
    expect(composerControls([model, thinking])).toEqual({ models: [model], reasoning: [thinking], modelConfig: [], other: [] });
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
      models: [model], reasoning: [reasoning], modelConfig: [speed, parameter], other: [custom],
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
    expect(composerControls([model, custom])).toEqual({ models: [model], reasoning: [], modelConfig: [], other: [custom] });
  });
});
