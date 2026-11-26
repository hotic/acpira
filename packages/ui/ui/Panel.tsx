import type { MouseEventHandler, ReactNode } from 'react';
import { Check, ChevronRight } from 'lucide-react';
import { cn } from './cn';

// Shared column geometry; reserve the check slot on every selectable row.
// descriptionClassName: overrides the caption's colour (a settings select dims its source captions further)
// `trail` stands in the check column instead of the check mark (a model row's arrow into its parameters)
export function OptionContent({ icon, children, description, descriptionClassName, extra, checked, checkSlot = false, trail }: {
  icon?: ReactNode; children: ReactNode; description?: string; descriptionClassName?: string; extra?: ReactNode; checked?: boolean; checkSlot?: boolean; trail?: ReactNode;
}) {
  return <>
    {icon && <span className="flex size-icon shrink-0 items-center justify-center [&_svg]:size-icon">{icon}</span>}
    <span className="flex min-w-0 flex-1 flex-col"><span className="truncate">{children}</span>
      {description && <span className={cn('truncate text-3 text-fg-3', descriptionClassName)}>{description}</span>}{extra}
    </span>
    {trail ?? (checked ? <Check className="size-icon shrink-0 text-fg-1" strokeWidth={2} /> : checkSlot && <span className="size-icon shrink-0" aria-hidden />)}
  </>;
}

export interface FooterAction {
  label: string;
  icon: ReactNode;
  onClick: MouseEventHandler<HTMLButtonElement>;
  disabled?: boolean;
}

export interface PanelBarProps {
  // Square button at the left edge (back to the previous page)
  lead?: FooterAction;
  // Text in the middle: a title / note, or an entry into another page when onClick is given (then it gets a trailing chevron). Text only — icons go in lead / action
  children?: ReactNode;
  onClick?: MouseEventHandler<HTMLButtonElement>;
  // Square button at the right edge
  action?: FooterAction;
  // Multiple square buttons at the right edge, left to right; only the last bleeds past the edge when no tail follows
  actions?: FooterAction[];
  // A small control at the right edge when the action slot needs more than a button (e.g. a picker)
  tail?: ReactNode;
}

// A --ctl-tall bar at either edge of a menu, one divider between it and the options area (modeled on Devin's agent menu).
// Layout is [lead] [text …spacer…] [action]: the text takes its natural width, so an entry highlights as a small pill rather than the whole bar.
// Text is row-sized (text-2), not caption-sized — the bar is part of the menu, not a footnote to it
function PanelBar({ edge, lead, children, onClick, action, actions, tail }: PanelBarProps & { edge: 'top' | 'bottom' }) {
  const text = 'flex h-ctl min-w-0 items-center gap-1 px-2 text-left text-2';
  const right = actions ?? (action ? [action] : []);
  return (
    <div className={cn('flex items-center gap-1 border-line', edge === 'top' ? 'mb-1 border-b pb-1' : 'mt-1 border-t pt-1')}>
      {/* The squares bleed so their glyphs stand on the rows' icon and text edges (tokens.css bar-bleed) */}
      {lead && <BarButton {...lead} className="-ms-bar-bleed" />}
      {children !== undefined && (onClick
        ? (
          <button type="button" onClick={onClick} className={cn(text, 'rounded-md text-fg-2 outline-none transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1')}>
            <span className="truncate">{children}</span>
            <ChevronRight className="size-3 shrink-0 text-fg-3" strokeWidth={1.75} />
          </button>
        )
        : <div className={cn(text, 'text-fg-1')}><span className="truncate">{children}</span></div>)}
      <span className="min-w-0 flex-1" />
      {tail}
      {right.length > 1
        ? (
          // Grouped header actions use compact squares (`size-ctl-sm`) so adjacent icons read as one tight cluster;
          // `pe-2` minus `-me-ctl-sm-bleed` keeps the trailing glyph on the rows' 8px right text edge at every density
          <span className="flex shrink-0 items-center gap-0.5 pe-2">
            {right.map((item, i) => <BarButton key={item.label} {...item} compact className={i === right.length - 1 ? '-me-ctl-sm-bleed' : undefined} />)}
          </span>
        )
        : right.map(item => <BarButton key={item.label} {...item} className="-me-bar-bleed" />)}
    </div>
  );
}

// Navigation bar of a sub-page: back on the left, title in the middle, one action on the right
export function PanelHeader(p: PanelBarProps) { return <PanelBar edge="top" {...p} />; }
// Footer of a menu: a note or an entry on the left, one action on the right
export function PanelFooter(p: PanelBarProps) { return <PanelBar edge="bottom" {...p} />; }

function BarButton({ label, icon, onClick, disabled, compact, className }: FooterAction & { compact?: boolean; className?: string }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      disabled={disabled}
      onClick={onClick}
      className={cn(
        'flex shrink-0 items-center justify-center rounded-md text-fg-3 outline-none transition-colors hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1 disabled:pointer-events-none disabled:opacity-40 [&>svg]:size-icon',
        compact ? 'size-ctl-sm' : 'size-ctl',
        className,
      )}
    >
      {icon}
    </button>
  );
}
