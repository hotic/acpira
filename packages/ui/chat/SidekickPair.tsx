import { useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { CircleFadingPlus, Plus } from 'lucide-react';
import { isUltraLevel, levelShort } from '@acpira/shared/composerControls';
import { t } from '../i18n';
import { cn } from '../ui/cn';
import { cssTimeVar } from '../ui/cssTime';
import { ModelMark } from './ModelMark';

export type PairRole = 'lead' | 'sidekick';

// What one side of the switch shows: the model (none for a sidekick not picked yet) and its level
export interface PairSide { name?: string; brand?: string; level?: string }

// The model panel's lead / sidekick switch (the built-in agent with a sidekick select). Left is the lead and right the
// sidekick, by position alone: no role words, which would need a glyph per language. The side being edited grows and spells
// out its model; the other folds to its mark and the narrow chip's short level, and the sidekick's mark carries a + badge.
// Picked in lab/sidekick-focus.preview.html (2026-10-21)
export function PairSwitch({ role, lead, sidekick, onRole }: { role: PairRole; lead: PairSide; sidekick: PairSide; onRole: (role: PairRole) => void }) {
  return (
    // Ruled off from the list above it, as the search line is from the list below it, and as tall: the same h-search-line
    // band (its rule included) taking back the popup's bottom padding as the search line takes back the top (-mb-1 / -mt-1),
    // with the track centred in it (4px either side of the default 26px track)
    <div className="-mb-1 mt-1 flex h-search-line shrink-0 flex-col justify-center border-t border-line">
      <div className="flex gap-0.5 rounded-md bg-hover p-0.5">
        <Side role="lead" side={lead} on={role === 'lead'} onClick={() => onRole('lead')} />
        <Side role="sidekick" side={sidekick} on={role === 'sidekick'} onClick={() => onRole('sidekick')} />
      </div>
    </div>
  );
}

function Side({ role, side, on, onClick }: { role: PairRole; side: PairSide; on: boolean; onClick: () => void }) {
  const roleName = t(role === 'lead' ? 'composer.lead' : 'composer.sidekick');
  const none = t('composer.sidekickNone');
  const title = t('composer.pairSide', { role: roleName, model: side.name ? [side.name, side.level].filter(Boolean).join(' ') : none });
  const [lastLevel, setLastLevel] = useState(side.level);
  if (side.level && side.level !== lastLevel) setLastLevel(side.level);
  const level = side.level ?? lastLevel;
  const showLevel = !!(side.name && side.level);
  const iconRef = useRef<HTMLSpanElement>(null);
  const nameRef = useRef<HTMLSpanElement>(null);
  const prevName = useRef(side.name);
  useLayoutEffect(() => {
    if (prevName.current === side.name) return;
    prevName.current = side.name;
    const el = nameRef.current;
    if (!el || el.closest('[data-motion="none"]') || window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) return;
    const duration = cssTimeVar(el, '--dur', 150);
    for (const target of [iconRef.current, el]) {
      target?.animate([{ opacity: 0, transform: 'translateY(2px)' }, { opacity: 1, transform: 'none' }], { duration, easing: 'ease-out' });
    }
  }, [side.name]);
  let icon: ReactNode = side.name && <ModelMark family={side.name} brand={side.brand} />;
  if (role === 'sidekick') icon = side.name
    ? <SidekickMark name={side.name} brand={side.brand} />
    // No sidekick yet: an empty, fading slot that reads as "add one" in any language
    : <CircleFadingPlus className="size-icon shrink-0 text-fg-3" strokeWidth={1.75} aria-hidden />;
  return (
    <button type="button" title={title} aria-label={title} aria-pressed={on} data-on={on ? '' : undefined} onClick={onClick}
      className={cn('pair-side flex h-ctl-sm min-w-0 items-baseline gap-1.5 rounded-sm px-2 text-left text-2 outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-fg-2',
        on ? 'bg-active text-fg-1' : 'text-fg-2 hover:text-fg-1')}>
      <span ref={iconRef} className="flex shrink-0 self-center">{icon}</span>
      <span className={cn('pair-name', !side.name && 'text-fg-3')}><span ref={nameRef}>{side.name ?? none}</span></span>
      <span className="pair-level" data-open={showLevel ? '' : undefined} aria-hidden={!showLevel || undefined}>
        <span className={cn('text-3', level && isUltraLevel(level) ? 'text-ultra' : 'text-fg-3')}>{level && (on ? level : levelShort(level))}</span>
      </span>
    </button>
  );
}

// The sidekick's mark with its + badge, as the folded side of PairSwitch shows it
function SidekickMark({ name, brand }: { name: string; brand?: string }) {
  return <span className="pair-plus"><ModelMark family={name} brand={brand} /><span className="pair-plus-badge" aria-hidden><Plus absoluteStrokeWidth strokeWidth={1.5} /></span></span>;
}

// Both sides' bodies in one box (`.pair-stack`): the lead's sets the panel's height and the sidekick's fills it from above, so
// neither a switch nor a pick resizes the open panel; the hidden side is inert. `rows` is how many rows the sidekick's list
// holds: a lead with a short list of its own (a CLI of four models) still leaves the box tall enough to pick from them
export function PairViews({ role, lead, sidekick, rows }: { role: PairRole; lead: ReactNode; sidekick: ReactNode; rows: number }) {
  return (
    <div className="pair-stack" style={{ '--pair-rows': rows } as CSSProperties}>
      <div className="pair-view" data-side="lead" data-on={role === 'lead' ? '' : undefined} inert={role !== 'lead'}>{lead}</div>
      <div className="pair-view" data-side="sidekick" data-on={role === 'sidekick' ? '' : undefined} inert={role !== 'sidekick'}>{sidekick}</div>
    </div>
  );
}
