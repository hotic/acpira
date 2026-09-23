import { describe, expect, it } from 'vitest';
import { arrangeAgents, launchable, moveAgent, pickDefaultAgent } from '../src/shared/agentOrder';
import { sanitizeSetting } from '../src/shared/settings';
import type { AgentInfo } from '../src/shared/transcript';

const list: AgentInfo[] = [
  { id: 'grok', name: 'Grok' },
  { id: 'devin', name: 'Devin' },
  { id: 'chatgpt', name: 'ChatGPT', external: true },
  { id: 'kimi', name: 'Kimi', available: false },
  { id: 'pi', name: 'Pi' },
];
const ids = (agents: AgentInfo[]) => agents.map(a => a.id);

describe('agent order', () => {
  it('puts saved ids first, keeps registry order for the rest and external entries last', () => {
    expect(ids(arrangeAgents(list, { order: [], disabled: [] }))).toEqual(['grok', 'devin', 'kimi', 'pi', 'chatgpt']);
    expect(ids(arrangeAgents(list, { order: ['pi', 'ghost', 'chatgpt', 'devin'], disabled: [] }))).toEqual(['pi', 'devin', 'grok', 'kimi', 'chatgpt']);
  });

  it('flags disabled agents without dropping them; external entries cannot be disabled', () => {
    const arranged = arrangeAgents(list, { order: [], disabled: ['devin', 'chatgpt'] });
    expect(arranged.find(a => a.id === 'devin')?.disabled).toBe(true);
    expect(arranged.find(a => a.id === 'chatgpt')?.disabled).toBeUndefined();
    expect(ids(launchable(arranged))).toEqual(['grok', 'kimi', 'pi']);
    expect(list.find(a => a.id === 'devin')?.disabled).toBeUndefined();
  });

  it('falls back from a disabled default to the first enabled installed agent', () => {
    const arranged = arrangeAgents(list, { order: ['kimi', 'pi'], disabled: ['grok'] });
    expect(pickDefaultAgent(arranged, 'devin')).toBe('devin');
    expect(pickDefaultAgent(arranged, 'grok')).toBe('pi');
    expect(pickDefaultAgent(arranged, 'custom-not-probed')).toBe('custom-not-probed');
    const allOff = arrangeAgents(list, { order: [], disabled: ['grok', 'devin', 'kimi', 'pi'] });
    expect(pickDefaultAgent(allOff, 'grok')).toBe('grok');
  });

  it('moves an id to a clamped position', () => {
    expect(moveAgent(['a', 'b', 'c'], 'c', 0)).toEqual(['c', 'a', 'b']);
    expect(moveAgent(['a', 'b', 'c'], 'a', 9)).toEqual(['b', 'c', 'a']);
    expect(moveAgent(['a', 'b', 'c'], 'b', -1)).toEqual(['b', 'a', 'c']);
    expect(moveAgent(['a', 'b'], 'x', 0)).toEqual(['a', 'b']);
  });

  it('sanitizes both settings to unique trimmed id lists', () => {
    expect(sanitizeSetting('agentOrder', [' pi ', 'pi', '', 3, 'grok'])).toEqual(['pi', 'grok']);
    expect(sanitizeSetting('disabledAgents', 'pi')).toEqual([]);
    expect(sanitizeSetting('disabledAgents', undefined)).toEqual([]);
  });
});
