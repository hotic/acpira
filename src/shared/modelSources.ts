import type { ConfigControl, SessionOption } from './transcript';

// What the host read off the agent's config: Grok aliases, Pi / OpenCode provider ids, or ALL for the single endpoint
// Codex / Claude route every model through
export type ModelSources = Record<string, NonNullable<SessionOption['source']>>;

export const ALL = '*';

// `provider/model` ids (Pi, OpenCode): the provider becomes the source, and the `Provider/` label the adapter put in front
// of the name moves into it. Already-sourced options are left alone, so a republish never strips twice.
function providerPrefixed(option: SessionOption, configured: ModelSources) {
  if (option.source) return;
  const slash = option.id.indexOf('/');
  if (slash < 1) return;
  const provider = option.id.slice(0, slash);
  const cut = option.name.indexOf('/');
  const label = cut > 0 && option.name.slice(0, cut).trim() && option.name.slice(cut + 1).trim() ? option.name.slice(0, cut).trim() : undefined;
  if (label) option.name = option.name.slice(cut + 1).trim();
  const known = configured[provider];
  option.source = known ? { ...known, name: label ?? known.name } : { id: provider, name: label ?? provider, kind: 'official' };
}

// Agent-specific aliases are interpreted here; the generic model UI treats IDs as opaque.
export function applyModelSources(agent: string, controls: ConfigControl[], configured: ModelSources = {}) {
  for (const control of controls) {
    if (control.category !== 'model') continue;
    for (const option of control.options) {
      if (agent === 'kimi') {
        const slash = option.id.indexOf('/');
        if (slash < 1) continue;
        const provider = option.id.slice(0, slash);
        option.source = { id: provider, name: provider, kind: provider === 'kimi-code' ? 'official' : 'custom' };
      } else if (agent === 'grok') {
        const source = configured[option.id];
        if (source) option.source = source;
        // Known built-in aliases from Grok's modelState; a configured endpoint takes precedence.
        else if (/^grok-\d/.test(option.id) || option.id === 'grok-build') option.source = { id: 'grok', name: 'Grok', kind: 'official' };
      } else if (agent === 'pi' || agent === 'opencode') {
        providerPrefixed(option, configured);
      } else if ((agent === 'codex' || agent === 'claude') && configured[ALL]) {
        option.source = configured[ALL];
      }
    }
  }
}
