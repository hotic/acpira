import { describe, expect, it } from 'vitest';
import { filterModels, prioritizeModels, setModelsVisible } from '../src/shared/modelCatalog';
import { familyHidden, groupModels, visibleOptions } from '../src/shared/models';
import type { SessionOption } from '../src/shared/transcript';

const options = (names: string[]): SessionOption[] => names.map(name => ({ id: name, name }));
const catalog = (names: string[]) => groupModels(options(names));

describe('model settings catalog', () => {
  it('prioritizes newest versions per series without comparing unrelated vendors', () => {
    const families = catalog(['Claude Opus 5', 'GLM-5.2', 'Claude Opus 5.5', 'Kimi K2.7', 'GLM-5.3', 'Kimi K3']);
    expect(prioritizeModels(families).map(family => family.name)).toEqual([
      'Claude Opus 5.5', 'GLM-5.3', 'Kimi K3', 'Claude Opus 5', 'GLM-5.2', 'Kimi K2.7',
    ]);
    expect(families[0]!.name).toBe('Claude Opus 5');
  });

  it('compares version components numerically and preserves unversioned aliases and sizes', () => {
    expect(prioritizeModels(catalog(['Opus 5.9', 'Automatic', 'Opus 5.10', 'Qwen 8B', 'Qwen 32B'])).map(family => family.name))
      .toEqual(['Opus 5.10', 'Automatic', 'Qwen 8B', 'Qwen 32B', 'Opus 5.9']);
    expect(prioritizeModels(catalog(['claude-opus-4-6', 'claude-opus-4-8']))[0]!.name).toBe('claude-opus-4-8');
  });

  it('keeps source identity and opaque variant ids intact', () => {
    const families = groupModels([
      { id: 'official-old', name: 'Claude Opus 5', source: { id: 'official', name: 'official', kind: 'official' } },
      { id: 'gateway-new', name: 'Claude Opus 5.5', source: { id: 'gateway', name: 'gateway', kind: 'custom' } },
      { id: 'official-new', name: 'Claude Opus 5.5', source: { id: 'official', name: 'official', kind: 'official' } },
    ]);
    expect(prioritizeModels(families).map(family => family.variants[0]!.id)).toEqual(['official-new', 'gateway-new', 'official-old']);
  });

  it('searches the full catalog by name, source and variant id without changing order', () => {
    const families = groupModels([{ id: 'asgard/opus-max', name: 'Claude Opus 5.5 Max', source: { id: 'asgard', name: 'asgard', kind: 'custom' } }, { id: 'auto', name: 'Automatic' }]);
    expect(filterModels(families, ' ASGARD opus ')).toEqual([families[0]]);
    expect(filterModels(families, 'opus-max')).toEqual([families[0]]);
    expect(filterModels(families, 'unmatched')).toEqual([]);
    expect(filterModels(families, '  ')).toEqual(families);
  });

  it('bulk switches every family and variant while retaining unknown preferences and the current value', () => {
    const choices = options(['Claude Opus 5 Low', 'Claude Opus 5 Max', 'GLM-5.3']);
    const families = groupModels(choices);
    const hidden = setModelsVisible(families, ['absent-model'], false);
    expect(families.every(family => familyHidden(family, hidden))).toBe(true);
    expect(visibleOptions(choices, hidden, choices[0]!.id)).toEqual([choices[0]]);
    expect(setModelsVisible(families, hidden, true)).toEqual(['absent-model']);
  });

  it('clears legacy name-only preferences across same-named sources', () => {
    const families = groupModels([
      { id: 'official/k3', name: 'K3', source: { id: 'official', name: 'official', kind: 'official' } },
      { id: 'gateway/k3', name: 'K3', source: { id: 'gateway', name: 'gateway', kind: 'custom' } },
    ]);
    expect(setModelsVisible(families, ['K3'], true)).toEqual([]);
    expect(setModelsVisible(families, ['K3'], false)).toEqual(families.map(family => family.key));
  });
});
