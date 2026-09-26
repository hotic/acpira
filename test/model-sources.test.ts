import { describe, expect, it } from 'vitest';
import { ALL, applyModelSources } from '../src/shared/modelSources';
import { findVariant, groupModels, setFamilyVisible, visibleOptions } from '../src/shared/models';
import type { ConfigControl } from '../src/shared/transcript';

describe('ACP model source adapters', () => {
  it('groups a Grok custom endpoint apart from the official models', () => {
    // What the engine reads off ~/.grok/config.toml for this config (rust/crates/acpira-host/tests/engine/registry.rs)
    const sources = { asgard: { id: 'asgard', name: 'asgard', kind: 'custom' as const } };
    const control: ConfigControl = { id: 'model', name: 'Model', category: 'model', options: [
      { id: 'grok-4.6', name: 'Grok 4.6' }, { id: 'asgard', name: 'Grok 4.6' }, { id: 'grok-build', name: 'grok-build' },
    ] };
    applyModelSources('grok', [control], sources);
    const families = groupModels(control.options);
    expect(families.map(f => f.sourceKind)).toEqual(['official', 'custom', 'official']);
    expect(findVariant(families[1]!, '', false, false)?.id).toBe('asgard');
    expect(visibleOptions(control.options, [families[0]!.key]).map(o => o.id)).toEqual(['asgard', 'grok-build']);
  });

  it('keeps future ACP groups distinct even when names and parameter tuples match', () => {
    const families = groupModels([
      { id: 'one', name: 'Model High', group: { id: 'provider-a', name: 'Provider A' } },
      { id: 'two', name: 'Model High', group: { id: 'provider-b', name: 'Provider B' } },
    ]);
    expect(families).toHaveLength(2);
    expect(findVariant(families[1]!, 'High', false, false)?.id).toBe('two');
  });

  it('uses opaque IDs to distinguish ambiguous options from an unknown ACP agent', () => {
    const families = groupModels([{ id: 'one', name: 'K3' }, { id: 'two', name: 'K3' }]);
    expect(families).toHaveLength(2);
    expect(families.map(f => f.source)).toEqual(['one', 'two']);
    expect(findVariant(families[1]!, '', false, false)?.id).toBe('two');
  });

  it('does not infer providers from opaque slash IDs belonging to other agents', () => {
    const control: ConfigControl = { id: 'model', name: 'Model', category: 'model', options: [{ id: 'opaque/value', name: 'Model High' }] };
    applyModelSources('devin', [control]);
    expect(control.options[0]!.source).toBeUndefined();
    expect(groupModels(control.options)[0]!.source).toBeUndefined();
  });
  it('moves the provider label of Pi / OpenCode names into the source, once', () => {
    const sources = { asgard: { id: 'asgard', name: 'asgard', kind: 'custom' as const } };
    const control: ConfigControl = { id: 'model', name: 'Model', category: 'model', options: [
      { id: 'asgard/kimi-k3', name: 'asgard/Kimi K3' }, { id: 'anthropic/claude-opus-5', name: 'anthropic/Claude Opus 5' },
    ] };
    applyModelSources('pi', [control], sources);
    applyModelSources('pi', [control], sources);
    expect(control.options.map(o => [o.name, o.source?.name, o.source?.kind])).toEqual([
      ['Kimi K3', 'asgard', 'custom'], ['Claude Opus 5', 'anthropic', 'official'],
    ]);
    const oc: ConfigControl = { id: 'model', name: 'Model', category: 'model', options: [{ id: 'asgard/glm-5.3', name: 'Asgard/GLM-5.3' }] };
    applyModelSources('opencode', [oc], sources);
    expect([oc.options[0]!.name, oc.options[0]!.source?.name]).toEqual(['GLM-5.3', 'Asgard']);
  });

  it('keeps a Provider/Name preference hidden once the provider became the source', () => {
    const control: ConfigControl = { id: 'model', name: 'Model', category: 'model', options: [
      { id: 'asgard/kimi-k3', name: 'asgard/Kimi K3' }, { id: 'asgard/glm-5.3', name: 'asgard/GLM-5.3' },
    ] };
    applyModelSources('pi', [control]);
    expect(visibleOptions(control.options, ['asgard/Kimi K3']).map(o => o.id)).toEqual(['asgard/glm-5.3']);
    const kimi = groupModels(control.options)[0]!;
    expect(setFamilyVisible(control.options, ['asgard/Kimi K3'], groupModels(control.options)[1]!.key, false)).toEqual(
      expect.arrayContaining([kimi.key]));
  });

  it('sources every Codex / Claude model from the single configured endpoint', () => {
    const control: ConfigControl = { id: 'model', name: 'Model', category: 'model', options: [{ id: 'opus', name: 'claude-opus-5.5' }] };
    applyModelSources('claude', [control]);
    expect(control.options[0]!.source).toBeUndefined();
    applyModelSources('claude', [control], { [ALL]: { id: 'gw.example', name: 'gw.example', kind: 'custom' } });
    expect(control.options[0]!.source?.kind).toBe('custom');
  });
});
