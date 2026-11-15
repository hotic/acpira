import { useEffect, useMemo, useState, type RefObject } from 'react';
import { Goal, SquareSlash, type LucideIcon } from 'lucide-react';
import type { MsgKey } from '@acpira/shared/i18n/core';
import type { SlashCommand } from '@acpira/shared/transcript';
import { presentCommand } from '@acpira/shared/commandPresentation';
import { getLocale, t } from '../i18n';
import { CompletionList } from './Completion';
import { matchCommands } from './slashCommands';

export { commandAt, commandHint, commandMarks, completeCommand, matchCommands, promptMarks, summonMarks, type CommandMark, type SlashSpan } from './slashCommands';

// Something the shell does itself, listed by name ahead of the agent's commands: entering a working mode, compacting.
// `id` is its English handle, matched beside the localized label so `/plan` finds it in any language
export interface HostAction { id: string; label: string; description?: string; icon: LucideIcon; run: () => void }
export type SlashHit = SlashCommand | HostAction;
export const isHostAction = (hit: SlashHit): hit is HostAction => 'run' in hit;

function matchActions(actions: readonly HostAction[], query: string): HostAction[] {
  const q = query.toLowerCase();
  if (!q) return [...actions];
  const of = (a: HostAction) => [a.label.toLowerCase(), a.id.toLowerCase()];
  const prefix = actions.filter(a => of(a).some(x => x.startsWith(q)));
  return [...prefix, ...actions.filter(a => !prefix.includes(a) && a.label.toLowerCase().includes(q))];
}

// Agent commands the shell knows well enough to list by a localized name, right after its own actions. Presentation
// only: a pick still writes `/name`, which is what goes to the agent
const NAMED: Record<string, { label: MsgKey; icon: LucideIcon }> = { goal: { label: 'composer.action.goal', icon: Goal } };
const named = (c: SlashCommand) => (Object.hasOwn(NAMED, c.name) ? NAMED[c.name] : undefined);

// Named commands first, each tier in the agent's order; a query also finds one by the start of its localized name
function withNamed(commands: readonly SlashCommand[], matched: SlashCommand[], query: string): SlashCommand[] {
  const q = query.toLowerCase();
  const byLabel = q ? commands.filter(c => !matched.includes(c) && named(c) && t(named(c)!.label).toLowerCase().startsWith(q)) : [];
  const all = [...byLabel, ...matched];
  return [...all.filter(c => named(c)), ...all.filter(c => !named(c))];
}

// Filters synchronously (the list is already in the session view) and keeps the active row in range; the row resets whenever the query changes
export function useSlashHits(commands: readonly SlashCommand[] | undefined, query: string | undefined, actions: readonly HostAction[] = NO_ACTIONS) {
  const locale = getLocale();
  const matched = useMemo(() => (query === undefined || !commands ? [] : withNamed(commands, matchCommands(commands, query, locale), query)), [commands, query, locale]);
  const hits: SlashHit[] = query === undefined ? matched : [...matchActions(actions, query), ...matched];
  const [active, setActive] = useState(0);
  useEffect(() => setActive(0), [query]);
  const index = active < hits.length ? active : 0;
  const move = (dir: 1 | -1) => setActive(i => (hits.length ? (i + dir + hits.length) % hits.length : 0));
  return { hits, active: index, setActive, move };
}
const NO_ACTIONS: readonly HostAction[] = [];

interface SlashListProps {
  anchor: RefObject<HTMLElement | null>;
  hits: SlashHit[];
  active: number;
  onHover: (index: number) => void;
  onPick: (hit: SlashHit) => void;
}

// The command list floating over the composer in the shared completion shell. A host action reads as its name and one
// line on what it does; an agent command as `/name` bright, the input hint faint beside it, the description trailing.
// Only ever shown with hits — without a match the slash stays ordinary text and Enter sends it as typed
export function SlashList({ anchor, hits, active, onHover, onPick }: SlashListProps) {
  return <CompletionList anchor={anchor} items={hits} active={active} keyOf={hit => (isHostAction(hit) ? `host:${hit.id}` : hit.name)} onHover={onHover} onPick={onPick}>
    {hit => {
      if (isHostAction(hit)) return <>
        <hit.icon className="size-icon shrink-0" strokeWidth={1.75} />
        <span className="shrink-0">{hit.label}</span>
        {hit.description && <span title={hit.description} className="min-w-0 flex-1 truncate text-3 text-fg-3">{hit.description}</span>}
      </>;
      const c = presentCommand(hit, getLocale());
      const known = named(hit);
      if (known) return <>
        <known.icon className="size-icon shrink-0" strokeWidth={1.75} />
        <span className="shrink-0">{t(known.label)}</span>
        {c.input?.hint && <span className="max-w-[30%] shrink-0 truncate font-mono text-mono text-fg-3">{c.input.hint}</span>}
        {c.description && <span title={c.description} className="min-w-0 flex-1 truncate text-3 text-fg-3">{c.description}</span>}
      </>;
      return <>
      <SquareSlash className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
      <span title={`/${c.name}`} className="max-w-[60%] shrink-0 truncate font-mono text-mono">/{c.name}</span>
      {c.input?.hint && <span className="max-w-[30%] shrink-0 truncate font-mono text-mono text-fg-3">{c.input.hint}</span>}
      {c.description && <span title={c.description} className="min-w-0 flex-1 truncate text-3 text-fg-3">{c.description}</span>}
    </>;
    }}
  </CompletionList>;
}
