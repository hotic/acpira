import { describe, expect, it } from 'vitest';
import type { PluginInfo, PluginsView } from '@acpira/shared/plugins';
import type { PluginCard } from '@acpira/shared/transcript';
import { cardState, shownCards } from './PluginCard';

const card = (snapshot: string, at: number): PluginCard => ({ id: snapshot, plugin: 'meadow', snapshot, name: 'Meadow', description: '', kind: 'skin', folder: '/p/meadow', at });
const view = (snapshot: string, committedAt: number): PluginsView =>
  ({ skin: 'meadow', safeMode: false, plugins: [{ id: 'meadow', enabled: true, snapshot, committedAt } as PluginInfo] }) as unknown as PluginsView;

describe('cardState', () => {
  it('offers an update made after the version in effect', () => {
    // v1 applied at 200, the agent then offered v2 at 300
    expect(cardState(card('v1', 100), view('v1', 200)).state).toBe('applied');
    expect(cardState(card('v2', 300), view('v1', 200)).state).toBe('pending');
  });

  it('marks a card whose version was superseded later', () => {
    // v2 applied at 400: the v1 card offered at 100 now has a newer version in use
    expect(cardState(card('v1', 100), view('v2', 400)).state).toBe('newer');
    expect(cardState(card('v2', 300), view('v2', 400)).state).toBe('applied');
  });

  it('marks a card whose plugin left the store', () => {
    const empty = { skin: '', safeMode: false, plugins: [] } as unknown as PluginsView;
    expect(cardState(card('v1', 100), empty).state).toBe('removed');
  });
});

describe('shownCards', () => {
  it('keeps the last card of a plugin offered again, and an earlier one applied from here', () => {
    const v1 = card('v1', 100);
    const v2 = card('v2', 200);
    const v3 = card('v3', 300);
    expect(shownCards([v1, v2, v3])).toEqual([v3]);
    const applied = { ...v1, undo: {} };
    expect(shownCards([applied, v2, v3])).toEqual([applied, v3]);
  });
});
