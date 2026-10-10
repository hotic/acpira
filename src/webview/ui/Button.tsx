import type { ButtonHTMLAttributes, ReactNode, Ref } from 'react';
import { ChevronDown } from 'lucide-react';
import { cn } from './cn';
import { cva } from 'class-variance-authority';

type Variant = 'primary' | 'secondary';

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  ref?: Ref<HTMLButtonElement>;
  variant?: Variant;
  kbd?: string;
}

// Two neutral tiers (picked in the buttons LAB, scheme "tonal"): primary is a tonal grey fill with strong text and a firmer outline, secondary is an outline that fills on hover.
// No inverted solid (that look belongs to the send button alone) and no colored buttons (accent is reserved for status dots)
const buttonVariants = cva('inline-flex h-ctl items-center gap-1.5 whitespace-nowrap rounded-md px-3 text-2 transition-colors', {
  variants: { variant: {
    primary: 'border border-line-strong bg-active text-fg-strong font-medium hover:bg-chip-hover focus-visible:bg-chip-hover',
    secondary: 'border border-line text-fg-2 hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1',
  } }, defaultVariants: { variant: 'secondary' },
});
const iconVariants = cva('inline-flex shrink-0 items-center justify-center rounded-md text-fg-2 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1 data-[popup-open]:bg-active data-[popup-open]:text-fg-1', {
  variants: { size: { sm: 'size-ctl-sm [&_svg]:size-icon', default: 'size-ctl [&_svg]:size-icon-ctl' } }, defaultVariants: { size: 'default' },
});

// All buttons share --ctl height, --r-md radius, and 12px horizontal padding
export function Button({ variant = 'secondary', kbd, className, children, ...rest }: ButtonProps) {
  return (
    <button
      type="button"
      className={cn(
        buttonVariants({ variant }),
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
// Two looks (modeled on Cursor's toolbar): solid is the filled pill the eye lands on first — one per toolbar, the mode; quiet is text until hovered, for everything else.
// Splitting the hierarchy this way keeps a long mode name from reading as "buttons crowding buttons" when it pushes its neighbours
type ChipVariant = 'quiet' | 'solid';

const chipVariants = cva('inline-flex h-ctl-sm min-w-0 items-center gap-1 rounded-sm px-1.5 text-3 transition-colors', {
  variants: { variant: {
    quiet: 'text-fg-2 hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1 data-[open]:bg-active data-[open]:text-fg-1 data-[popup-open]:bg-active data-[popup-open]:text-fg-1',
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
  // the caret goes first either way (as in Devin's own composer, which has none). The title keeps the full name. Needs an @container ancestor
  narrow?: 'icon' | 'text';
  children: ReactNode;
}

// A badge inside a Chip's `tags`: --text-4 on the active fill; `ultra` tints it, an icon (Fast's bolt) leads the text.
// `short` replaces the text below the sm container tier, together with the caret going ('' keeps only the icon), so a
// narrow toolbar shows [M] [⚡] instead of clipping the badges away
export function ChipTag({ tone, icon, short, children }: { tone?: 'ultra'; icon?: ReactNode; short?: string; children: ReactNode }) {
  const compact = short !== undefined;
  return (
    <span className={cn('inline-flex h-4 shrink-0 items-center gap-0.5 rounded-xs px-1 text-4 leading-none', tone === 'ultra' ? 'bg-ultra/15 text-ultra' : 'bg-active text-fg-2')}>
      {icon}
      <span className={cn(compact && '@max-sm:hidden')}>{children}</span>
      {short && <span className="hidden @max-sm:inline">{short}</span>}
    </span>
  );
}

export function Chip({ className, children, variant = 'quiet', caret = true, icon, meta, tags, narrow, ...rest }: ChipProps) {
  return (
    <button
      type="button"
      className={cn(chipVariants({ variant }), className)}
      {...rest}
    >
      {icon && <span className={cn('flex shrink-0 items-center [&_svg]:size-icon', variant === 'quiet' && 'text-fg-3')}>{icon}</span>}
      <span className={cn('truncate text-left', tags ? 'shrink-0' : 'grow', narrow === 'icon' && '@max-sm:hidden')}>
        {children}
        {meta && <span className="text-fg-3"> {meta}</span>}
      </span>
      {tags && <span className="flex h-4 min-w-0 flex-wrap items-center gap-0.5 overflow-hidden">{tags}</span>}
      {caret && <ChevronDown className={cn('size-3 shrink-0 text-fg-3', narrow && '@max-sm:hidden')} strokeWidth={1.75} />}
    </button>
  );
}
