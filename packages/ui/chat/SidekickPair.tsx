import type { ReactNode } from 'react';
import { CircleFadingPlus, Plus } from 'lucide-react';
import { isUltraLevel, levelShort } from '@shared/composerControls';
import { t } from '../i18n';
import { ChipTag } from '../ui/Button';
import { cn } from '../ui/cn';
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
    <div className="mb-1 flex gap-0.5 rounded-md bg-hover p-0.5">
      <Side role="lead" side={lead} on={role === 'lead'} onClick={() => onRole('lead')} />
      <Side role="sidekick" side={sidekick} on={role === 'sidekick'} onClick={() => onRole('sidekick')} />
    </div>
  );
}

function Side({ role, side, on, onClick }: { role: PairRole; side: PairSide; on: boolean; onClick: () => void }) {
  const roleName = t(role === 'lead' ? 'composer.lead' : 'composer.sidekick');
  const none = t('composer.sidekickNone');
  const title = t('composer.pairSide', { role: roleName, model: side.name ? [side.name, side.level].filter(Boolean).join(' ') : none });
  const mark = side.name && <ModelMark family={side.name} brand={side.brand} />;
  let icon: ReactNode = mark;
  if (role === 'sidekick') icon = side.name
    ? <span className="pair-plus">{mark}<span className="pair-plus-badge" aria-hidden><Plus absoluteStrokeWidth strokeWidth={1.5} /></span></span>
    // No sidekick yet: an empty, fading slot that reads as "add one" in any language
    : <CircleFadingPlus className="size-icon shrink-0 text-fg-3" strokeWidth={1.75} aria-hidden />;
  return (
    <button type="button" title={title} aria-label={title} aria-pressed={on} data-on={on ? '' : undefined} onClick={onClick}
      className={cn('pair-side flex h-ctl-sm min-w-0 items-center gap-1.5 rounded-sm px-2 text-left text-2 outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-fg-2',
        on ? 'bg-active text-fg-1' : 'text-fg-2 hover:text-fg-1')}>
      {icon}
      <span className={cn('pair-name', !side.name && 'text-fg-3')}><span>{side.name ?? none}</span></span>
      {side.name && side.level && <span className="flex shrink-0">
        <ChipTag tone={isUltraLevel(side.level) ? 'ultra' : undefined}>{on ? side.level : levelShort(side.level)}</ChipTag>
      </span>}
    </button>
  );
}

// Both sides' bodies in one box (`.pair-stack`): the lead's sets the panel's height and the sidekick's fills it from above, so
// neither a switch nor a pick resizes the open panel; the hidden side is inert
export function PairViews({ role, lead, sidekick }: { role: PairRole; lead: ReactNode; sidekick: ReactNode }) {
  return (
    <div className="pair-stack">
      <div className="pair-view" data-side="lead" data-on={role === 'lead' ? '' : undefined} inert={role !== 'lead'}>{lead}</div>
      <div className="pair-view" data-side="sidekick" data-on={role === 'sidekick' ? '' : undefined} inert={role !== 'sidekick'}>{sidekick}</div>
    </div>
  );
}
