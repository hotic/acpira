import { useEffect, useRef, useState, type RefObject } from 'react';
import { FileText, Image as ImageIcon } from 'lucide-react';
import type { FileHit } from '@shared/protocol';
import { imageMimeOf } from '@shared/attachments';
import { t } from '../i18n';
import { CompletionList } from './Completion';

// An @ token under the caret: where it starts in the text and what has been typed after it
export interface MentionSpan {
  start: number;
  query: string;
}

// The @ must sit at the start or after whitespace, and the query runs up to the caret without whitespace; anything else is a plain @ (emails, decorators)
export function mentionAt(text: string, caret: number): MentionSpan | undefined {
  const m = /(^|\s)@([^\s@]*)$/.exec(text.slice(0, caret));
  return m ? { start: caret - m[2]!.length - 1, query: m[2]! } : undefined;
}

// Fetches hits for the current query (debounced, stale replies dropped) and keeps the active row in range.
// `ready` says the hits belong to the current query — until then the previous list is still shown but must not be picked from
export function useMentionHits(query: string | undefined, search: (q: string) => Promise<FileHit[]>) {
  // query is the one the list answers; undefined until the first reply, so an empty query is not mistaken for "already answered"
  const [hits, setHits] = useState<{ query?: string; list: FileHit[] }>({ list: [] });
  const [active, setActive] = useState(0);
  const seq = useRef(0);
  useEffect(() => {
    // Closing invalidates whatever is in flight, so a late reply cannot refill the list
    const mine = ++seq.current;
    if (query === undefined) { setHits({ list: [] }); return; }
    const t = setTimeout(() => {
      void search(query).then(r => { if (seq.current === mine) { setHits({ query, list: r }); setActive(0); } });
    }, 60);
    return () => clearTimeout(t);
  }, [query, search]);
  const list = hits.list;
  const move = (dir: 1 | -1) => setActive(i => (list.length ? (i + dir + list.length) % list.length : 0));
  return { hits: list, ready: query !== undefined && hits.query === query, active, setActive, move };
}

interface MentionListProps {
  anchor: RefObject<HTMLElement | null>;
  hits: FileHit[];
  active: number;
  empty: boolean;
  onHover: (index: number) => void;
  onPick: (hit: FileHit) => void;
}

// The file list floating over the composer: one row per hit (name bright, directory faint) in the shared completion shell
export function MentionList({ anchor, hits, active, empty, onHover, onPick }: MentionListProps) {
  return <CompletionList anchor={anchor} items={hits} active={active} keyOf={h => h.uri} empty={empty ? t('mention.noFiles') : undefined} onHover={onHover} onPick={onPick}>
    {h => {
      const cut = h.path.lastIndexOf('/');
      const Icon = imageMimeOf(h.path) ? ImageIcon : FileText;
      return <>
        <Icon className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
        <span className="truncate font-mono text-mono">{cut >= 0 ? h.path.slice(cut + 1) : h.path}</span>
        {cut >= 0 && <span className="truncate text-3 text-fg-3">{h.path.slice(0, cut)}</span>}
      </>;
    }}
  </CompletionList>;
}
