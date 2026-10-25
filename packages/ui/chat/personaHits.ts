import type { FileHit } from '@acpira/shared/protocol';

// A cross-harness subagent (settings → Subagents) offered in the @ list; picking it writes `@name ` into the text, which
// the agent reads as "summon this one" (the ask_agent tool description says so). Its hit travels as a FileHit with this scheme
export const PERSONA_SCHEME = 'acpira-agent:';
export interface MentionPersona {
  id: string;
  name: string;
  agent: string;
  // Faint second part of the row: the CLI's name and the persona's model, set apart by spacing rather than a separator
  // (for a group, its members' names)
  cli?: string;
  model?: string;
  // The persona's reasoning effort as the composer chip names it; shown apart from the model, whose id may carry an
  // unrelated level (Devin's `gemini-4-argon-medium` runs at whatever its effort select says)
  effort?: string;
  // A group of personas (settings → Orchestration): one @name summons every member
  group?: boolean;
}
export const personaOfHit = (hit: FileHit, personas: MentionPersona[] = []) =>
  hit.uri.startsWith(PERSONA_SCHEME) ? personas.find(p => p.id === hit.uri.slice(PERSONA_SCHEME.length)) : undefined;

// The personas whose name or id contains the query, as hits leading the file list
export function personaHits(personas: MentionPersona[], query: string): FileHit[] {
  const q = query.toLowerCase();
  return personas.filter(p => p.name.toLowerCase().includes(q) || p.id.includes(q)).map(p => ({ uri: `${PERSONA_SCHEME}${p.id}`, path: p.name }));
}
