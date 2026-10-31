import { describe, expect, it } from 'vitest';
import type { PluginInfo, PluginsView } from '@acpira/shared/plugins';
import type { PluginCard } from '@acpira/shared/transcript';
import { cardState } from './PluginCard';

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
});
