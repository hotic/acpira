import { describe, expect, it } from 'vitest';
import { syncListedModels, type ProviderModel } from '../providers';

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

  it('does not touch models the listing does not name', () => {
    const kept: ProviderModel[] = [model('keep-me')];
    expect(syncListedModels(kept, [model('sol', true)])).toBeUndefined();
    // A listing entry with an unknown flag stays a no-op for configured models
    expect(syncListedModels(kept, [model('sol')])).toBeUndefined();
  });
});
