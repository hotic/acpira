import type { FileHit } from '@shared/protocol';

// A cross-harness subagent (settings → Subagents) offered in the @ list; picking it writes `@name ` into the text, which
// the agent reads as "summon this one" (the ask_agent tool description says so). Its hit travels as a FileHit with this scheme
export const PERSONA_SCHEME = 'acpira-agent:';
export interface MentionPersona {
  id: string;
  name: string;
  agent: string;
  // Faint second part of the row: the CLI and model
  meta?: string;
}
export const personaOfHit = (hit: FileHit, personas: MentionPersona[] = []) =>
  hit.uri.startsWith(PERSONA_SCHEME) ? personas.find(p => p.id === hit.uri.slice(PERSONA_SCHEME.length)) : undefined;

// The personas whose name or id contains the query, as hits leading the file list
export function personaHits(personas: MentionPersona[], query: string): FileHit[] {
  const q = query.toLowerCase();
  return personas.filter(p => p.name.toLowerCase().includes(q) || p.id.includes(q)).map(p => ({ uri: `${PERSONA_SCHEME}${p.id}`, path: p.name }));
}
