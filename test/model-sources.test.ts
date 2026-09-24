import { describe, expect, it } from 'vitest';
import { applyModelSources } from '../src/shared/modelSources';
import { findVariant, groupModels, visibleOptions } from '../src/shared/models';
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
});
