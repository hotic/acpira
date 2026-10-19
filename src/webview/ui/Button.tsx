import type { ButtonHTMLAttributes, ReactNode, Ref } from 'react';
import { ChevronDown } from 'lucide-react';
import { cn } from './cn';
import { cva } from 'class-variance-authority';

type Variant = 'primary' | 'secondary';

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  ref?: Ref<HTMLButtonElement>;
  variant?: Variant;
  // A destructive action (Remove): red text that stays red on hover, over a faint red fill
  tone?: 'danger';
  kbd?: string;
}

// Two neutral tiers (picked in the buttons LAB, scheme "tonal"): primary is a tonal grey fill with strong text and a firmer outline, secondary is an outline that fills on hover.
// No inverted solid (that look belongs to the send button alone) and no colored buttons (accent is reserved for status dots); the danger tone only tints a secondary's text.
// Hover is one clear step on both tiers: primary goes from --active to --chip-hover, secondary fills with --active and firms its outline
// (--hover alone, a list row's step, was too faint to read on a button)
const buttonVariants = cva('inline-flex h-ctl items-center gap-1.5 whitespace-nowrap rounded-md px-3 text-2 transition-colors', {
  variants: {
    variant: {
      primary: 'border border-line-strong bg-active text-fg-strong font-medium hover:bg-chip-hover focus-visible:bg-chip-hover',
      secondary: 'border border-line text-fg-2 hover:border-line-strong hover:bg-active hover:text-fg-1 focus-visible:border-line-strong focus-visible:bg-active focus-visible:text-fg-1',
    },
    tone: {
      danger: 'text-danger hover:border-danger/40 hover:bg-danger/10 hover:text-danger focus-visible:border-danger/40 focus-visible:bg-danger/10 focus-visible:text-danger',
    },
  }, defaultVariants: { variant: 'secondary' },
});
// Icon buttons hover to --active, like the session list's row actions; an open menu keeps the same pressed fill
const iconVariants = cva('inline-flex shrink-0 items-center justify-center rounded-md text-fg-2 transition-colors hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1 data-[popup-open]:bg-active data-[popup-open]:text-fg-1', {
  variants: { size: { sm: 'size-ctl-sm [&_svg]:size-icon', default: 'size-ctl [&_svg]:size-icon-ctl' } }, defaultVariants: { size: 'default' },
});

// All buttons share --ctl height, --r-md radius, and 12px horizontal padding
export function Button({ variant = 'secondary', tone, kbd, className, children, ...rest }: ButtonProps) {
  return (
    <button
      type="button"
      className={cn(
        buttonVariants({ variant, tone }),
        className,
      )}
      {...rest}
    >
      {children}
      {kbd && <kbd className="font-sans text-3 opacity-60">{kbd}</kbd>}
    </button>
  );
}

// Square icon button: --ctl × --ctl, icon --icon-ctl; as a menu trigger pass data-open to keep the pressed look
export function IconButton({ className, children, size = 'default', ...rest }: ButtonHTMLAttributes<HTMLButtonElement> & { ref?: Ref<HTMLButtonElement>; children: ReactNode; size?: 'default' | 'sm' }) {
  return (
    <button
      type="button"
      className={cn(
        iconVariants({ size }),
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
}

// Selector that opens a menu (session title / mode / model / agent): --ctl-sm tall and --r-sm radius, snug around its text like Cursor's
// toolbar pills rather than filling the row; truncatable text, trailing arrow. Sets data-open while open to keep the pressed look.
// The arrow turns over while the menu is open (data-open / Base UI's data-popup-open on the trigger), so a click visibly lands
// even when the panel opens upward, away from a downward arrow
// Two looks (modeled on Cursor's toolbar): solid is the filled pill the eye lands on first — one per toolbar, the mode; quiet is text until hovered, for everything else.
// Splitting the hierarchy this way keeps a long mode name from reading as "buttons crowding buttons" when it pushes its neighbours
type ChipVariant = 'quiet' | 'solid';

const chipVariants = cva('inline-flex h-ctl-sm min-w-0 items-center gap-1 rounded-sm px-1.5 text-3 transition-colors', {
  variants: { variant: {
    quiet: 'text-fg-2 hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1 data-[open]:bg-active data-[open]:text-fg-1 data-[popup-open]:bg-active data-[popup-open]:text-fg-1',
    solid: 'bg-chip text-fg-1 hover:bg-chip-hover focus-visible:bg-chip-hover data-[open]:bg-chip-hover data-[popup-open]:bg-chip-hover',
  } }, defaultVariants: { variant: 'quiet' },
});

export interface ChipProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  ref?: Ref<HTMLButtonElement>;
  variant?: ChipVariant;
  caret?: boolean;
  // Small mark before the text (agent vendor mark / mode glyph), --icon sized
  icon?: ReactNode;
  // Faint trailing text after the label (a model's params), truncates together with it
  meta?: string;
  // Small badges after the label (ChipTag: a model's effort / Ultra / Fast). The label keeps its full width; badges that do not fit
  // wrap onto a clipped second line, so the trailing ones give way first
  tags?: ReactNode;
  // What survives below the sm container tier (the toolbar of a 380 sidebar has ~324px): 'icon' keeps just the icon, 'text' keeps just the label;
  // the caret goes first either way (as in Devin's own composer, which has none). The title keeps the full name. Needs an @container ancestor.
  // 'icon-xl' keeps just the icon below the xl tier already: several live-context chips sharing the composer toolbar
  narrow?: 'icon' | 'icon-xl' | 'text';
  children: ReactNode;
}

// A badge inside a Chip's `tags`: --text-4 on the active fill; `ultra` tints it, an icon (Fast's bolt) leads the text.
// `short` replaces the text below the sm container tier, together with the caret going ('' keeps only the icon), so a
// narrow toolbar shows [M] [⚡] instead of clipping the badges away. `small` is the quieter badge next to faint row text
// (a summoned subagent's effort): a shorter box in the row's own faint tone
export function ChipTag({ tone, icon, short, small, children }: { tone?: 'ultra'; icon?: ReactNode; short?: string; small?: boolean; children: ReactNode }) {
  const compact = short !== undefined;
  return (
    <span className={cn('inline-flex shrink-0 items-center gap-0.5 rounded-xs px-1 text-4 leading-none', small ? 'h-3.5' : 'h-4',
      tone === 'ultra' ? 'bg-ultra/15 text-ultra' : cn('bg-active', small ? 'text-fg-3' : 'text-fg-2'))}>
      {icon}
      <span className={cn(TRIM_CAPS, compact && '@max-sm:hidden')}>{children}</span>
      {short && <span className={cn(TRIM_CAPS, 'hidden @max-sm:inline')}>{short}</span>}
    </span>
  );
}

// The badge text's box is trimmed to cap height and baseline, so centring it centres the capitals; untrimmed, Inter's
// ascender space sits them about 1px low in the 16px box. Hosts without text-box support keep the untrimmed look
const TRIM_CAPS = '[text-box:trim-both_cap_alphabetic]';

// Turns a trigger's down arrow over while its menu is open; the trigger carries `group/chip`. Shared by the hand-built
// triggers (SelectRow, the session list filter) so every dropdown arrow behaves alike
export const CARET_TURN = 'transition-transform duration-(--dur-open) ease-out group-data-[open]/chip:rotate-180 group-data-[popup-open]/chip:rotate-180';

export function Chip({ className, children, variant = 'quiet', caret = true, icon, meta, tags, narrow, ...rest }: ChipProps) {
  return (
    <button
      type="button"
      className={cn('group/chip', chipVariants({ variant }), className)}
      {...rest}
    >
      {icon && <span className={cn('flex shrink-0 items-center [&_svg]:size-icon', variant === 'quiet' && 'text-fg-3')}>{icon}</span>}
      <span className={cn('truncate text-left', tags ? 'shrink-0' : 'grow', narrow === 'icon' && '@max-sm:hidden', narrow === 'icon-xl' && '@max-xl:hidden')}>
        {children}
        {meta && <span className="text-fg-3"> {meta}</span>}
      </span>
      {tags && <span className="flex h-4 min-w-0 flex-wrap items-center gap-0.5 overflow-hidden">{tags}</span>}
      {caret && <ChevronDown className={cn('size-3 shrink-0 text-fg-3', CARET_TURN, narrow && '@max-sm:hidden')} strokeWidth={1.75} />}
    </button>
  );
}
