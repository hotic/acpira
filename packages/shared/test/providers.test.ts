import { describe, expect, it } from 'vitest';
import { syncListedFast, type ProviderModel } from '../providers';

// An endpoint listing may only have named the fast tier on some models; a model's own fast setting wins over the listing
describe('syncListedFast', () => {
  const model = (id: string, fastListed?: boolean | null): ProviderModel => ({
    id, enabled: true, input: ['text'], thinking: 'auto', efforts: [], sampling: {}, estimated: [], fastListed,
  });

  it('moves the listed flags onto configured models, keeping everything else', () => {
    const listed: ProviderModel[] = [model('sol', true), model('astra', false)];
    const next = syncListedFast([model('sol'), model('astra')], listed);
    expect(next).toEqual([model('sol', true), model('astra', false)]);
    // Unchanged models come back as the same references; only moved entries are new
    expect(syncListedFast([model('sol', true), model('astra', false)], listed)).toBeUndefined();
  });

  it('clears a flag the listing withdrew', () => {
    const listed: ProviderModel[] = [model('astra')];
    const next = syncListedFast([model('sol', true), model('astra', true)], listed);
    expect(next).toEqual([model('sol', true), model('astra')]);
  });

  it('is undefined when nothing moved or nothing was listed', () => {
    const kept: ProviderModel[] = [model('sol'), model('astra')];
    expect(syncListedFast(kept, [model('sol'), model('astra')])).toBeUndefined();
    expect(syncListedFast(kept)).toBeUndefined();
    expect(syncListedFast([], [model('sol', true)])).toBeUndefined();
  });

  it('does not touch models the listing does not name', () => {
    const kept: ProviderModel[] = [model('keep-me')];
    expect(syncListedFast(kept, [model('sol', true)])).toBeUndefined();
    // A listing entry with an unknown flag stays a no-op for configured models
    expect(syncListedFast(kept, [model('sol')])).toBeUndefined();
  });
});
