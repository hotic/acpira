import { useEffect, useMemo, useState, type RefObject } from 'react';
import { SquareSlash } from 'lucide-react';
import type { SlashCommand } from '@shared/transcript';
import { presentCommand } from '@shared/commandPresentation';
import { getLocale } from '../i18n';
import { CompletionList } from './Completion';
import { matchCommands } from './slashCommands';

export { commandAt, commandHint, matchCommands, type SlashSpan } from './slashCommands';

// Filters synchronously (the list is already in the session view) and keeps the active row in range; the row resets whenever the query changes
export function useSlashHits(commands: readonly SlashCommand[] | undefined, query: string | undefined) {
  const locale = getLocale();
  const hits = useMemo(() => (query === undefined || !commands ? [] : matchCommands(commands, query, locale)), [commands, query, locale]);
  const [active, setActive] = useState(0);
  useEffect(() => setActive(0), [query]);
  const index = active < hits.length ? active : 0;
  const move = (dir: 1 | -1) => setActive(i => (hits.length ? (i + dir + hits.length) % hits.length : 0));
  return { hits, active: index, setActive, move };
}

interface SlashListProps {
  anchor: RefObject<HTMLElement | null>;
  hits: SlashCommand[];
  active: number;
  onHover: (index: number) => void;
  onPick: (command: SlashCommand) => void;
}

// The command list floating over the composer in the shared completion shell: `/name` bright, the input hint faint beside it, the description trailing.
// Only ever shown with hits — without a match the slash stays ordinary text and Enter sends it as typed
export function SlashList({ anchor, hits, active, onHover, onPick }: SlashListProps) {
  return <CompletionList anchor={anchor} items={hits} active={active} keyOf={c => c.name} onHover={onHover} onPick={onPick}>
    {command => {
      const c = presentCommand(command, getLocale());
      return <>
      <SquareSlash className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
      <span title={`/${c.name}`} className="max-w-[60%] shrink-0 truncate font-mono text-mono">/{c.name}</span>
      {c.input?.hint && <span className="max-w-[30%] shrink-0 truncate font-mono text-mono text-fg-3">{c.input.hint}</span>}
      {c.description && <span title={c.description} className="min-w-0 flex-1 truncate text-3 text-fg-3">{c.description}</span>}
    </>;
    }}
  </CompletionList>;
}
