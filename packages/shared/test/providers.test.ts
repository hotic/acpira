import { describe, expect, it } from 'vitest';
import { moveModel, orderModels, syncListedModels, type ProviderModel } from '../providers';

// Refresh endpoint metadata without replacing explicit model settings.
describe('syncListedModels', () => {
  const model = (id: string, fastListed?: boolean | null): ProviderModel => ({
    id, enabled: true, input: ['text'], thinking: 'auto', efforts: [], sampling: {}, estimated: [], fastListed,
  });

  it('moves the listed flags onto configured models, keeping everything else', () => {
    const listed: ProviderModel[] = [model('sol', true), model('astra', false)];
    const next = syncListedModels([model('sol'), model('astra')], listed);
    expect(next).toEqual([model('sol', true), model('astra', false)]);
    // Unchanged models come back as the same references; only moved entries are new
    expect(syncListedModels([model('sol', true), model('astra', false)], listed)).toBeUndefined();
  });

  it('clears a flag the listing withdrew', () => {
    const listed: ProviderModel[] = [model('astra')];
    const next = syncListedModels([model('sol', true), model('astra', true)], listed);
    expect(next).toEqual([model('sol', true), model('astra')]);
  });

  it('is undefined when nothing moved or nothing was listed', () => {
    const kept: ProviderModel[] = [model('sol'), model('astra')];
    expect(syncListedModels(kept, [model('sol'), model('astra')])).toBeUndefined();
    expect(syncListedModels(kept)).toBeUndefined();
    expect(syncListedModels([], [model('sol', true)])).toBeUndefined();
  });

  it('replaces guessed effort levels with the endpoint scale and clears only its estimate', () => {
    const guessed = { ...model('deepseek-v4.1-flash'), efforts: ['low', 'medium', 'high'], estimated: ['context', 'efforts'], fast: false };
    const untouched = model('keep-me');
    const listed = [{ ...model(guessed.id, true), efforts: ['low', 'high', 'max'] }];
    const next = syncListedModels([guessed, untouched], listed)!;
    expect(next[0]).toEqual({ ...guessed, efforts: ['low', 'high', 'max'], estimated: ['context'], fastListed: true });
    expect(next[1]).toBe(untouched);
    expect(guessed.efforts).toEqual(['low', 'medium', 'high']);
    expect(guessed.estimated).toEqual(['context', 'efforts']);
    expect(syncListedModels(next, listed)).toBeUndefined();
  });

  it('confirms an unchanged scale too', () => {
    const saved = { ...model('deepseek'), efforts: ['low', 'high', 'max'], estimated: ['efforts'] };
    expect(syncListedModels([saved], [{ ...saved, estimated: [] }])).toEqual([{ ...saved, estimated: [] }]);
  });

  it('keeps manually saved levels and thinking-off while updating listed fast tiers', () => {
    for (const saved of [
      { ...model('deepseek'), efforts: ['low', 'medium', 'high'] },
      { ...model('deepseek'), thinking: 'off' as const, estimated: ['efforts'] },
      model('deepseek'),
    ]) {
      expect(syncListedModels([saved], [{ ...model(saved.id, true), efforts: ['low', 'high', 'max'] }]))
        .toEqual([{ ...saved, fastListed: true }]);
    }
  });

  it('does not replace a guess with another guess or an absent scale', () => {
    const saved = { ...model('deepseek'), efforts: ['low', 'medium', 'high'], estimated: ['efforts'] };
    for (const fresh of [model(saved.id), { ...model(saved.id), efforts: ['low', 'high', 'max'], estimated: ['efforts'] }]) {
      expect(syncListedModels([saved], [fresh])).toBeUndefined();
    }
  });

  it('replaces guessed image input when the endpoint confirms it and keeps explicit choices', () => {
    const guessed = { ...model('gpt-6.2-astra'), efforts: ['low', 'medium', 'high'], estimated: ['context', 'input', 'efforts'] };
    const listed = [{ ...model(guessed.id), input: ['text', 'image'], efforts: ['low', 'medium', 'high', 'xhigh', 'max'] }];
    const next = syncListedModels([guessed], listed)!;
    expect(next[0]).toEqual({ ...guessed, input: ['text', 'image'], efforts: ['low', 'medium', 'high', 'xhigh', 'max'], estimated: ['context'] });
    expect(syncListedModels(next, listed)).toBeUndefined();
    // Confirming an unchanged text-only list still clears the estimate
    const textOnly = { ...model('o3-mini'), estimated: ['input'] };
    expect(syncListedModels([textOnly], [model('o3-mini')])).toEqual([{ ...textOnly, estimated: [] }]);
    // Another unconfirmed guess leaves it alone
    expect(syncListedModels([textOnly], [{ ...model('o3-mini'), input: ['text', 'image'], estimated: ['input'] }])).toBeUndefined();
    // Explicitly saved text-only stays untouched even if the endpoint advertises images
    const confirmed = model('gpt-6.2-astra');
    expect(syncListedModels([confirmed], [{ ...confirmed, input: ['text', 'image'] }])).toBeUndefined();
  });

  it('does not touch models the listing does not name', () => {
    const kept: ProviderModel[] = [model('keep-me')];
    expect(syncListedModels(kept, [model('sol', true)])).toBeUndefined();
    // A listing entry with an unknown flag stays a no-op for configured models
    expect(syncListedModels(kept, [model('sol')])).toBeUndefined();
  });
});

describe('moveModel / orderModels', () => {
  const model = (id: string): ProviderModel => ({
    id, enabled: true, input: ['text'], thinking: 'auto', efforts: [], sampling: {}, estimated: [],
  });

  it('moves a model within bounds and returns the original array on no-op', () => {
    const list = [model('a'), model('b'), model('c')];
    expect(moveModel(list, 'a', 2).map(m => m.id)).toEqual(['b', 'c', 'a']);
    expect(moveModel(list, 'c', 0).map(m => m.id)).toEqual(['c', 'a', 'b']);
    expect(moveModel(list, 'b', 99).map(m => m.id)).toEqual(['a', 'c', 'b']);
    expect(moveModel(list, 'b', -5).map(m => m.id)).toEqual(['b', 'a', 'c']);
    expect(moveModel(list, 'a', 0)).toBe(list);
    expect(moveModel(list, 'missing', 1)).toBe(list);
  });

  it('orders models by ids while keeping unlisted models and ignoring stale or duplicate ids', () => {
    const list = [model('a'), model('b'), model('c'), model('d')];
    expect(orderModels(list, ['c', 'gone', 'a', 'c']).map(m => m.id)).toEqual(['c', 'a', 'b', 'd']);
  });
});
