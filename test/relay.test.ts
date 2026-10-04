import { describe, expect, it } from 'vitest';
import { personaSlug, sanitizePersonas } from '../src/shared/subagents';
import { sanitizeSetting } from '../src/shared/settings';
import { personaHits, personaOfHit } from '../src/webview/chat/personaHits';
import fixture from './fixtures/personas-sanitize.json';

// Cross-harness subagents: the persona list mirrors acpira_shared::subagents::sanitize_personas (relay/roster.rs tests the Rust side)
describe('subagent personas', () => {
  it('drop unnamed or agentless entries, clip text and derive unique slug ids', () => {
    const out = sanitizePersonas([
      { name: 'Codex Review', agent: 'codex', model: ' gpt-6 ', mode: 'consult', when: 'review diffs' },
      { name: 'Codex Review', agent: 'codex', mode: 'work', enabled: false },
      { name: '', agent: 'codex' },
      { name: '快手', agent: 'opencode', mode: 'nonsense' },
      'garbage',
    ]);
    expect(out.map(p => p.id)).toEqual(['codex-review', 'codex-review-2', 'agent']);
    expect(out[0]).toMatchObject({ model: 'gpt-6', mode: 'consult', enabled: true });
    expect(out[1]).toMatchObject({ mode: 'work', enabled: false });
    expect(out[2]!.mode).toBe('consult');
    expect(personaSlug('  Claude  Planner! ')).toBe('claude-planner');
    expect(sanitizeSetting('subagents', 'nope')).toEqual([]);
  });

  // The same fixture runs through acpira_shared::subagents::sanitize_personas (persona_tests)
  it('sanitize the shared boundary fixture exactly like the Rust side', () => {
    expect(sanitizePersonas(fixture.input)).toEqual(fixture.output);
  });

  it('lead the @ list by name or id and resolve back from their hit', () => {
    const personas = [{ id: 'codex-review', name: 'Codex Review', agent: 'codex' }, { id: 'composer', name: 'Composer 快手', agent: 'opencode' }];
    const hits = personaHits(personas, 'co');
    expect(hits.map(h => h.path)).toEqual(['Codex Review', 'Composer 快手']);
    expect(personaHits(personas, '快手').map(h => h.path)).toEqual(['Composer 快手']);
    expect(personaOfHit(hits[1]!, personas)?.id).toBe('composer');
    expect(personaOfHit({ uri: 'file:///a.ts', path: 'a.ts' }, personas)).toBeUndefined();
  });
});
