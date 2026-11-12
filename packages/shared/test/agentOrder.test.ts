import { describe, expect, it } from 'vitest';
import { agentsForFolder, arrangeAgents, launchable, moveAgent, pickDefaultAgent } from '../agentOrder';
import { sanitizeSetting } from '../settings';
import type { AgentInfo } from '../transcript';

const list: AgentInfo[] = [
  { id: 'grok', name: 'Grok' },
  { id: 'devin', name: 'Devin' },
  { id: 'kimi', name: 'Kimi', available: false },
  { id: 'pi', name: 'Pi' },
];
const ids = (agents: AgentInfo[]) => agents.map(a => a.id);

describe('agent order', () => {
  it('puts saved ids first and keeps registry order for the rest', () => {
    expect(ids(arrangeAgents(list, { order: [], disabled: [] }))).toEqual(['grok', 'devin', 'kimi', 'pi']);
    expect(ids(arrangeAgents(list, { order: ['pi', 'ghost', 'devin'], disabled: [] }))).toEqual(['pi', 'devin', 'grok', 'kimi']);
  });

  it('flags disabled agents without dropping them', () => {
    const arranged = arrangeAgents(list, { order: [], disabled: ['devin'] });
    expect(arranged.find(a => a.id === 'devin')?.disabled).toBe(true);
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

  it('falls back from a default whose CLI is missing to the first enabled installed agent', () => {
    expect(pickDefaultAgent(arrangeAgents(list, { order: ['kimi', 'pi'], disabled: [] }), 'kimi')).toBe('pi');
    // Nothing installed to take over: the missing default stays, so its error card names it
    const none: AgentInfo[] = [{ id: 'kimi', name: 'Kimi', available: false }, { id: 'pi', name: 'Pi', available: false }];
    expect(pickDefaultAgent(none, 'kimi')).toBe('kimi');
  });

  it('uses the executing machine for SSH availability without changing local order or enabled choices', () => {
    const local: AgentInfo[] = [
      { id: 'acpira', name: 'Acpira', available: true },
      { id: 'codex', name: 'Codex', available: false },
      { id: 'claude', name: 'Claude', available: true, disabled: true },
    ];
    const host: AgentInfo[] = [
      { id: 'claude', name: 'Claude', available: false },
      { id: 'codex', name: 'Codex', available: true, disabled: true },
      { id: 'acpira', name: 'Acpira', available: true },
    ];
    const offered = agentsForFolder(local, 'ssh://mini/work', host);
    expect(ids(offered)).toEqual(ids(local));
    expect(launchable(offered).map(a => [a.id, a.available])).toEqual([['acpira', true], ['codex', true]]);
    expect(offered[2]).toMatchObject({ available: false, disabled: true });
    expect(local[1]?.available).toBe(false);
    expect(host[1]?.disabled).toBe(true);
    expect(agentsForFolder(local, '/work', host)).toBe(local);
    expect(agentsForFolder(local, undefined, host)).toBe(local);
    // Opted-in agents run here even when the host lacks them; everyone else still uses the host's probe.
    const mixed = agentsForFolder(local, 'ssh://mini/work', host, ['claude']);
    expect(mixed[2]).toBe(local[2]);
    expect(mixed[1]?.available).toBe(true);
  });

  it('does not mistake missing local CLIs for missing remote CLIs on older peers', () => {
    const local: AgentInfo[] = [{ id: 'codex', name: 'Codex', available: false }];
    expect(agentsForFolder(local, 'ssh://mini/work', undefined)[0]?.available).toBeUndefined();
    // A peer that did send its registry but lacks this agent is definitive, unlike an older peer.
    expect(agentsForFolder(local, 'ssh://mini/work', [])[0]?.available).toBe(false);
  });

  it('defaults to the built-in agent', () => {
    expect(sanitizeSetting('defaultAgent', undefined)).toBe('acpira');
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
