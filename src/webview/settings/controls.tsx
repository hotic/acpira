import { createContext, useContext, useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type MouseEvent, type ReactNode } from 'react';
import { BookText, Brain, ChevronRight, Combine, ExternalLink, Image as ImageIcon, Search, X, Zap } from 'lucide-react';
import { cn } from '../ui/cn';
import { Card } from '../ui/Card';
import { Chip, IconButton } from '../ui/Button';
import { Dialog } from '../ui/Dialog';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { Shimmer } from '../ui/Shimmer';
import { StableText } from '../ui/StableText';
import { Switch as SwitchPrimitive } from '../ui/Switch';
import { t } from '../i18n';

// Settings use a navigation column beside one content column. Page and section headings belong to the content.

export function PageHeader({ title, action }: { title: ReactNode; action?: ReactNode }) {
  return (
    <header className="flex min-h-ctl items-center gap-pad">
      <h1 className="m-0 min-w-0 flex-1 text-(length:--text-h) leading-(--text-h-lh) font-medium text-fg-1 [overflow-wrap:anywhere]">{title}</h1>
      {action && <div className="flex shrink-0 items-center">{action}</div>}
    </header>
  );
}

// Page body: one column of blocks; the editor host centres it at the content measure, the sidebar just fills
export function Page({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn('mx-auto flex w-full max-w-(--content-w) flex-col gap-(--section-gap) px-page py-pad-y', className)}>{children}</div>;
}

// Heading of a top-level section, several of which stack on one page; the Section labels below sit one level under it
export function SectionHead({ children, count, action, className }: { children: ReactNode; count?: number; action?: ReactNode; className?: string }) {
  return (
    <div className={cn('flex min-h-ctl items-center gap-gap', className)}>
      <h2 className="m-0 flex min-w-0 flex-1 items-baseline gap-2 text-(length:--text-h) leading-(--text-h-lh) font-medium text-fg-1">
        <span className="truncate">{children}</span>
        {count !== undefined && <Count n={count} />}
      </h2>
      {action}
    </div>
  );
}

export function Count({ n, className }: { n: number; className?: string }) {
  return <span className={cn('shrink-0 text-2 font-normal text-fg-2 tabular-nums', className)}>{n}</span>;
}

export interface SectionProps {
  title?: ReactNode;
  desc?: ReactNode;
  count?: number;
  // A mark before the title column (a source's glyph), sized to the two-line header
  lead?: ReactNode;
  // A small status mark between the title and the count (a Dot with its label)
  status?: ReactNode;
  // One small action (a quiet Chip / IconButton), at the right of the title line
  action?: ReactNode;
  // Children are already Cards (several groups), not rows to be wrapped in one Group
  cards?: boolean;
  children: ReactNode;
}

// Section copy shares the card's outer edge; only content inside the card receives its inset.
export function SectionDescription({ children }: { children: ReactNode }) {
  return <p className="m-0 text-2 text-fg-2 [overflow-wrap:anywhere]">{children}</p>;
}

// Titled subgroups own one surface. Their lists inherit it and use separators instead of nested cards.
const InsetGroupContext = createContext(false);

// The rules inside a card run from card edge to card edge, like a clickable row's hover fill (`rowHover`): a hovered row is
// a closed cell between two rules. Inset rules left the fill spilling past their ends and running into the rule above.
// `rowRules` draws one above every row but the first, `topRule` one above a block (a card's list under its head, a
// footer, a group after another); the card clips both at its border
export const rowRules = '[&>*+*]:relative [&>*+*]:after:pointer-events-none [&>*+*]:after:absolute [&>*+*]:after:top-0 [&>*+*]:after:-inset-x-pad [&>*+*]:after:h-px [&>*+*]:after:bg-line';
export const topRule = 'relative before:pointer-events-none before:absolute before:top-0 before:-inset-x-pad before:h-px before:bg-line';

// A text field on a settings card (inline forms)
export const inputBox = 'min-w-0 rounded-md border border-line bg-chip px-3 text-2 text-fg-1 outline-none placeholder:text-fg-3 focus:border-line-strong';

// Settings rows share typography and vertical padding. Detail rows reserve two text lines; wrapped content can grow.
// Inside a Group (or as a section header) the surface owns the horizontal inset, so the row drops its own.
const GroupContext = createContext(false);
const settingRow = 'min-h-(--setting-row) py-(--setting-row-pad) text-2 data-[detail]:min-h-(--setting-row-detail)';
function useSettingRow() {
  return cn(settingRow, !useContext(GroupContext) && 'px-pad');
}

export function Section({ title, desc, count, lead, status, action, cards, children }: SectionProps) {
  const inset = title !== undefined || !!action;
  const hasContent = children !== null && children !== undefined;
  const content = cards ? <div className={cn('flex flex-col', !inset && 'gap-pad')}>{children}</div> : <Group>{children}</Group>;
  if (!inset) return (
    <div className="flex flex-col gap-2">
      {desc && <SectionDescription>{desc}</SectionDescription>}
      {content}
    </div>
  );
  return (
    <Card className="acp-section-panel flex flex-col overflow-hidden px-pad shadow-none">
      <ListHead title={title} desc={desc} count={count} lead={lead} status={status} action={action} />
      {hasContent && <InsetGroupContext.Provider value={true}>
        <div className={topRule}>{content}</div>
      </InsetGroupContext.Provider>}
    </Card>
  );
}

// The head of a titled list: an optional lead mark, the title with its status and count, an optional second line, and
// the actions at the right. A Section card's header, and a group header inside a shared card (the model lists' groups).
// Without a lead the actions wrap under the copy when narrow; with one the row stays on one line (the copy shrinks), so
// the mark never ends up alone above its title
export function ListHead({ title, desc, count, lead, status, action }: Pick<SectionProps, 'title' | 'desc' | 'count' | 'lead' | 'status' | 'action'>) {
  return (
    <div className={cn(settingRow, 'flex items-center gap-gap', !lead && 'flex-wrap')} data-detail={!!desc || undefined}>
      {lead && <span className="flex size-lead shrink-0 items-center justify-center text-fg-2 [&_svg]:size-lead">{lead}</span>}
      <div className={cn('min-w-0', lead ? 'flex-1' : 'flex-[1_1_var(--setting-header-copy)]')}>
        <h3 className="m-0 flex items-baseline gap-2 text-2 font-normal text-fg-1">
          <span className="min-w-0 [overflow-wrap:anywhere]">{title}</span>
          {status}
          {count !== undefined && <Count n={count} />}
        </h3>
        {desc && <SectionDescription>{desc}</SectionDescription>}
      </div>
      {action}
    </div>
  );
}

// A search field in the shared box style; `className` sizes it in its row
export function SearchBox({ value, onChange, label, className }: { value: string; onChange: (v: string) => void; label: string; className?: string }) {
  return (
    <div className={cn('flex h-ctl min-w-0 items-center gap-2 rounded-md border border-line bg-chip px-3 focus-within:border-line-strong', className)}>
      <Search className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} aria-hidden />
      <input type="search" value={value} onChange={e => onChange(e.target.value)} aria-label={label} placeholder={label}
        className="min-w-0 flex-1 border-0 bg-transparent text-2 text-fg-1 outline-none placeholder:text-fg-3" />
    </div>
  );
}

// The first row inside a model list's card, the same on every agent's page: the search field, then an optional control
// (the master switch) at the right edge, where the rows' switches sit
export function SearchRow({ value, onChange, children }: { value: string; onChange: (v: string) => void; children?: ReactNode }) {
  return (
    <div className="flex min-w-0 items-center gap-gap py-pad-y">
      <SearchBox value={value} onChange={onChange} label={t('settings.models.search')} className="flex-1" />
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  );
}

// The badges a model row can carry, in their column order. Each is one muted icon in a fixed slot (no words beside it);
// the words are its label and tooltip
export type ModelTagKind = 'thinking' | 'fusion' | 'fast' | 'image' | 'long';
const MODEL_TAG_ORDER: ModelTagKind[] = ['thinking', 'fusion', 'fast', 'image', 'long'];
const MODEL_TAG_ICON: Record<ModelTagKind, ReactNode> = {
  thinking: <Brain strokeWidth={1.75} />,
  fusion: <Combine strokeWidth={1.75} />,
  fast: <Zap strokeWidth={1.75} />,
  image: <ImageIcon strokeWidth={1.75} />,
  long: <BookText strokeWidth={1.75} />,
};
// A row's badges: kind → its words
export type ModelTags = Partial<Record<ModelTagKind, string>>;

// One model on one compact line, shared by the built-in agent's sources and the other agents' visibility lists: mark,
// name (an optional marker after it), the badges packed against the row's own controls on the right, then those
// controls. The full description is the name's tooltip. `onOpen` makes the name a button stretched over the whole row
// (the controls stay above it) that opens the row's editor; its hover lights the row from card edge to card edge
export function ModelLine({ lead, name, marker, hint, tags, dim, trailing, onOpen, open, openLabel }: {
  lead: ReactNode; name: string; marker?: ReactNode; hint?: string; tags: ModelTags; dim?: boolean; trailing: ReactNode;
  onOpen?: () => void; open?: boolean; openLabel?: string;
}) {
  const label = <><span className="min-w-0 truncate">{name}</span>{marker}</>;
  const kinds = MODEL_TAG_ORDER.filter(k => tags[k]);
  return (
    <ItemRow lead={lead} dim={dim}
      className={cn(onOpen && 'relative isolate before:pointer-events-none before:absolute before:inset-y-0 before:-inset-x-pad before:-z-10 before:transition-colors has-[[data-model-open]:hover]:before:bg-hover has-[[data-model-open]:focus-visible]:before:bg-hover',
        onOpen && open && 'before:bg-hover')}
      title={onOpen
        ? <button type="button" data-model-open onClick={onOpen} aria-haspopup="dialog" aria-expanded={open} aria-label={openLabel} title={hint}
            className="flex w-full min-w-0 items-baseline gap-2 text-left outline-none after:absolute after:inset-y-0 after:-inset-x-pad">{label}</button>
        : <span className="flex min-w-0 items-baseline gap-2" title={hint}>{label}</span>}
      trailing={<span className="relative flex items-center gap-gap">
        {kinds.length > 0 && <span className="flex items-center gap-1.5 text-fg-3">
          {kinds.map(k => <span key={k} role="img" aria-label={tags[k]} title={tags[k]} className="flex size-icon items-center justify-center [&_svg]:size-icon">{MODEL_TAG_ICON[k]}</span>)}
        </span>}
        {trailing}
      </span>} />
  );
}

// A small text action for a Section's title line: quiet Chip, no caret. Its box (and hover fill) ends on the cards' right edge
// like the page header's action, rather than overhanging it
export function SectionAction({ icon, onClick, children, title, disabled }: { icon?: ReactNode; onClick: () => void; children: ReactNode; title?: string; disabled?: boolean }) {
  return <Chip caret={false} icon={icon} onClick={onClick} title={title} disabled={disabled} className="shrink-0">{children}</Chip>;
}

export function Group({ className, children, embedded = false }: { className?: string; children: ReactNode; embedded?: boolean }) {
  const inheritedInset = useContext(InsetGroupContext);
  const inset = embedded || inheritedInset;
  const Container = inset ? 'div' : Card;
  return <GroupContext.Provider value={true}>
    <Container className={cn('flex flex-col', rowRules, !inset && 'overflow-hidden px-pad shadow-none', className)}>{children}</Container>
  </GroupContext.Provider>;
}

// The settings pages' dialog frame, used by the add-model list, the source form and the model editor (the TextPeek /
// SubagentGraph overlay): a scrim over the settings shell, a titled box with a close button, the body, and an optional
// footer bar. A click on the scrim closes. It shares the menus' z-30 layer: a select inside portals its list after it
// into the same layer and so paints above it
export function SettingsDialog({ title, onClose, footer, children }: { title: string; onClose: () => void; footer?: ReactNode; children: ReactNode }) {
  const scrim = (e: MouseEvent) => { if (e.target === e.currentTarget) onClose(); };
  return <Dialog.Root open onOpenChange={next => { if (!next) onClose(); }}>
    <Dialog.Portal>
      <Dialog.Popup onClick={scrim} className="absolute inset-0 z-30 flex items-center justify-center bg-scrim p-pad">
        <div className="flex max-h-full w-pop-xl max-w-full flex-col overflow-hidden rounded-lg border border-line bg-bg-1 shadow-pop">
          <header className="flex shrink-0 items-center gap-gap border-b border-line py-gap pr-gap pl-pad">
            <Dialog.Title className="m-0 min-w-0 flex-1 truncate text-2 font-medium text-fg-1">{title}</Dialog.Title>
            <IconButton size="sm" title={t('common.close')} aria-label={t('common.close')} onClick={onClose} className="text-fg-2"><X strokeWidth={1.5} /></IconButton>
          </header>
          {children}
          {footer && <div className="flex shrink-0 items-center gap-2 border-t border-line px-pad py-gap">{footer}</div>}
        </div>
      </Dialog.Popup>
    </Dialog.Portal>
  </Dialog.Root>;
}

// A dialog form's field: its label above the control (the source form, the model editor); `hint` is the label's tooltip
export function FormField({ label, hint, children, className }: { label: string; hint?: string; children: ReactNode; className?: string }) {
  return (
    <div className={cn('flex min-w-0 flex-col gap-1', className)}>
      <span className="text-3 text-fg-2" title={hint}>{label}</span>
      {children}
    </div>
  );
}

// One setting: label + description on the left, the control on the right. `stack` puts the control under the text (wide controls)
export function Field({ label, desc, htmlFor, stack, children }: { label?: ReactNode; desc?: ReactNode; htmlFor?: string; stack?: boolean; children?: ReactNode }) {
  return (
    <div className={cn(useSettingRow(), 'flex flex-wrap gap-pad', stack ? 'flex-col' : 'items-center')} data-detail={!!desc || undefined}>
      <div className={cn('min-w-0', stack ? 'w-full' : 'flex-[1_1_var(--setting-header-copy)]')}>
        {label !== undefined && <label htmlFor={htmlFor} className="block text-2 text-fg-1">{label}</label>}
        {desc && <div className="text-2 text-fg-2 [overflow-wrap:anywhere]">{desc}</div>}
      </div>
      {/* max-w-full: once the row wraps, a long Select label truncates inside the card instead of overflowing it */}
      {children !== undefined && <div className={cn('flex max-w-full shrink-0 items-center gap-2', stack && 'w-full min-w-0')}>{children}</div>}
    </div>
  );
}

// A fact about the agent: name on the left, value at the right edge.
export function FactRow({ label, children }: { label: ReactNode; children: ReactNode }) {
  return (
    <div className={cn(useSettingRow(), 'flex items-center gap-pad')}>
      <span className="shrink-0 text-2 text-fg-2">{label}</span>
      <span className="flex min-w-0 flex-1 items-center justify-end gap-1.5 text-right text-2 text-fg-1">{children}</span>
    </div>
  );
}

// A faint single-line row inside a Group (empty states, notes)
export function Note({ children, shimmer }: { children: ReactNode; shimmer?: boolean }) {
  return <div className={cn(useSettingRow(), 'flex items-center text-fg-2')}><Shimmer active={!!shimmer}>{children}</Shimmer></div>;
}

// description: a caption under the label in the list only (the field shows the label alone)
export interface Option<V extends string> { value: V; label: string; icon?: ReactNode; hint?: string; disabled?: boolean; description?: string }

// Dropdown select: a bordered Chip that opens the shared Menu; radio semantics, the current value gets the check.
// Sized like NumberField (--ctl tall, --r-md) so the two form controls line up in a column. The field is as wide as its
// longest option (every label sits stacked in one grid cell, only the current one visible), so a pick never resizes it,
// and its list is exactly the field's width, like a native select. A className width (w-full in a form dialog) wins;
// the list follows it. The list is painted in the field's own material (`fieldTone`), and the field keeps its resting
// fill while open, so the two read as one control instead of a lighter field over a darker menu
export function Select<V extends string>({ options, value, onChange, label, className }: { options: Option<V>[]; value: V; onChange: (v: V) => void; label: string; className?: string }) {
  const cur = options.find(o => o.value === value);
  // Rows with a caption get the model panels' two-line padding, all of them so the list keeps one rhythm
  const twoLine = options.some(o => o.description);
  const trigger = useRef<HTMLButtonElement>(null);
  const [tone, setTone] = useState<CSSProperties>();
  return <DropdownMenu.Root onOpenChange={open => { if (open) setTone(fieldTone(trigger.current)); }}>
    <DropdownMenu.Trigger render={<Chip ref={trigger} aria-label={label} icon={cur?.icon}
      className={cn('h-ctl min-w-(--ctl-w) max-w-full gap-2 rounded-md border border-line bg-hover px-3 text-2 text-fg-1 hover:border-line-strong hover:bg-active data-[popup-open]:border-line-strong data-[popup-open]:bg-hover', className)}>
      {/* pe-1: a list row's chrome (popup border and padding, row padding, check) is a few px wider than the field's
          (border, padding, caret), so the longest label still fits its row unclipped */}
      <StableText className="pe-1" itemClassName="truncate" current={value}
        items={[...options.map(o => ({ key: o.value, text: o.label })), ...(cur ? [] : [{ key: value, text: value }])]} />
    </Chip>} />
    <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align="start" width="anchor"><DropdownMenu.Popup style={tone}>
      <DropdownMenu.RadioGroup value={value} className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
        {options.map(o => <DropdownMenu.RadioItem key={o.value} value={o.value} title={o.hint} disabled={o.disabled} onClick={() => onChange(o.value)} className={twoLine ? 'min-h-0 py-1.5' : undefined}>
          <OptionContent icon={o.icon} description={o.description} descriptionClassName="text-fg-3/70" checked={o.value === value} checkSlot={!!cur}>{o.label}</OptionContent>
        </DropdownMenu.RadioItem>)}
      </DropdownMenu.RadioGroup>
    </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
  </DropdownMenu.Root>;
}

// The field's look as one opaque paint for its list: the field's translucent fill (`--hover`) layered over the first
// opaque surface behind it (a card, a dialog), and the field's border colour. Read when the list opens: the list is
// portaled to the shell root, so it cannot inherit the card's or the dialog's colours, and the menu palette's own
// background (`--menu-bg`) is darker than a card
export function fieldTone(el: HTMLElement | null): CSSProperties | undefined {
  if (!el) return undefined;
  let surface: string | undefined;
  for (let p = el.parentElement; p && !surface; p = p.parentElement) {
    const bg = getComputedStyle(p).backgroundColor;
    if (alphaOf(bg) >= 1) surface = bg;
  }
  if (!surface) return undefined;
  const style = getComputedStyle(el);
  const fill = style.getPropertyValue('--hover').trim();
  return { background: fill ? `linear-gradient(${fill}, ${fill}), ${surface}` : surface, borderColor: style.borderTopColor };
}

// A computed colour's alpha: `rgb(…)` is opaque, `rgba(r, g, b, a)` and the `color(srgb … / a)` form colour-mix
// computes to carry it, `transparent` is none
function alphaOf(color: string): number {
  if (color === 'transparent') return 0;
  const slash = /\/\s*([\d.]+%?)\s*\)$/.exec(color);
  const comma = /^rgba\((?:[^,]+,){3}\s*([\d.]+%?)\s*\)$/.exec(color);
  const a = slash?.[1] ?? comma?.[1];
  if (a === undefined) return 1;
  return a.endsWith('%') ? parseFloat(a) / 100 : parseFloat(a);
}

export function Switch({ checked, onChange, label, disabled }: { checked: boolean; onChange: (on: boolean) => void; label: string; disabled?: boolean }) {
  return <SwitchPrimitive checked={checked} aria-label={label} disabled={disabled} onCheckedChange={onChange} />;
}

// Numeric field committing on blur / Enter; shows `unit` after the box. Local text state so half-typed values don't round-trip through the host.
// Out-of-range input snaps back to the current value rather than being clamped, so a typo never silently lands on a bound
export function NumberField({ value, onCommit, min, max, step, unit, label }: { value: number; onCommit: (v: number) => void; min?: number; max?: number; step?: number; unit?: string; label: string }) {
  const [text, setText] = useState(String(value));
  useEffect(() => { setText(String(value)); }, [value]);
  const commit = () => {
    const n = Number(text);
    if (!Number.isFinite(n) || (min !== undefined && n < min) || (max !== undefined && n > max)) { setText(String(value)); return; }
    if (n !== value) onCommit(n);
  };
  const onKey = (e: KeyboardEvent<HTMLInputElement>) => { if (e.key === 'Enter') (e.target as HTMLInputElement).blur(); if (e.key === 'Escape') setText(String(value)); };
  return (
    <label className="inline-flex h-ctl items-center gap-2 rounded-md border border-line bg-hover px-3 text-2 text-fg-1 transition-colors focus-within:bg-active">
      <input
        type="number"
        inputMode="numeric"
        aria-label={label}
        value={text}
        min={min}
        max={max}
        step={step}
        onChange={e => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={onKey}
        className="w-(--num-w) min-w-0 bg-transparent text-right tabular-nums outline-none [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
      />
      {unit && <span className="text-2 text-fg-2">{unit}</span>}
    </label>
  );
}

// Status dot before a status line: ok / off, or warn (needs attention). With a label it is announced and carries the
// label as its tooltip; without one it only repeats the text beside it
export function Dot({ ok, warn, label }: { ok: boolean; warn?: boolean; label?: string }) {
  const tone = ok ? 'bg-ok' : warn ? 'bg-warn' : 'bg-fg-3';
  if (label) return <span role="img" aria-label={label} title={label} className={cn('inline-block size-(--dot) shrink-0 self-center rounded-full', tone)} />;
  return <span aria-hidden className={cn('inline-block size-(--dot) shrink-0 rounded-full', tone)} />;
}

// Path display lives in a DOM-free module so the shared test suites can load it; re-exported for the pages
import { shortPath } from './paths';
export { pathSep, shortPath } from './paths';

export interface ItemRowProps {
  lead?: ReactNode;
  title: ReactNode;
  desc?: ReactNode;
  // A line under title / desc spanning the text column (quota bars); not truncated
  extra?: ReactNode;
  trailing?: ReactNode;
  // The row opens something (a file, a folder, a page): a click anywhere on it but its own controls opens, and a ↗ glyph
  // stays at the right edge as the cue and the keyboard stop
  onOpen?: () => void;
  // Keep metadata aligned in lists containing both openable and missing files.
  reserveOpen?: boolean;
  // The whole row is the button (navigation): a chevron at the right edge
  onClick?: () => void;
  // Greyed out as a whole (disabled server, missing file)
  dim?: boolean;
  // A lead mark that fills the whole lead slot instead of the one-line icon size, for rows whose desc makes them two lines tall
  bigLead?: boolean;
  className?: string;
}

// Full-width hover fill for a clickable row inside an inset Group: the row keeps the group's width (its text stays on the
// inset), while `::before` paints (and takes pointer hits) from card edge to card edge, between the rules above and below.
// One rule for every card: a row lights up only when a click anywhere on it does something (navigation, open, a row-shaped
// text button); rows that merely hold controls or information stay still
export const rowHover = 'relative isolate before:absolute before:inset-y-0 before:-inset-x-pad before:-z-10 before:transition-colors enabled:hover:before:bg-hover focus-visible:before:bg-hover';
// The same fill for an open row, which is a div (it may hold buttons of its own)
const openHover = 'relative isolate cursor-pointer before:absolute before:inset-y-0 before:-inset-x-pad before:-z-10 before:transition-colors hover:before:bg-hover';
const OWN_CONTROL = 'button, a, input, textarea, select, label, [role="switch"], [role="checkbox"]';

export function ItemRow({ lead, title, desc, extra, trailing, onOpen, reserveOpen, onClick, dim, bigLead, className }: ItemRowProps) {
  const Tag = onClick ? 'button' : 'div';
  const opens = !onClick && onOpen;
  // A click on one of the row's own controls (remove, adopt, …) stays theirs; anywhere else on the row opens
  const openFrom = opens ? (e: MouseEvent<HTMLElement>) => {
    const hit = (e.target as Element).closest(OWN_CONTROL);
    if (!hit || !e.currentTarget.contains(hit)) onOpen();
  } : undefined;
  return (
    <Tag
      {...(onClick ? { type: 'button' as const, onClick } : { onClick: openFrom })}
      data-detail={!!desc || undefined}
      className={cn(
        useSettingRow(), 'group/row flex w-full items-center gap-gap text-left',
        onClick && rowHover,
        opens && openHover,
        className,
      )}
    >
      {lead !== undefined && <span className={cn('flex size-lead shrink-0 items-center justify-center text-fg-2', bigLead ? '[&_svg]:size-lead' : '[&_svg]:size-icon', dim && 'opacity-60')}>{lead}</span>}
      <span className="flex min-w-0 flex-1 flex-col">
        <span className={cn('truncate text-2', dim ? 'text-fg-2' : 'text-fg-1')}>{title}</span>
        {desc && <span className="truncate text-2 text-fg-2">{desc}</span>}
        {extra}
      </span>
      {trailing && <span className="flex shrink-0 items-center gap-2">{trailing}</span>}
      {(onOpen || reserveOpen) && (
        // Zero height: the button overflows the slot evenly, so reserving it never makes the row taller than its neighbours
        <span className="-mr-1.5 flex h-0 w-ctl shrink-0 items-center justify-center" aria-hidden={!onOpen || undefined}>
          {/* Always shown as the row's cue; the pointer goes through to the row, the keyboard still stops here */}
          {onOpen && <IconButton title={t('common.open')} aria-label={t('common.open')} onClick={onOpen}
            className={opens ? 'pointer-events-none text-fg-3 group-hover/row:text-fg-2' : 'text-fg-2 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100'}>
            <ExternalLink strokeWidth={1.5} />
          </IconButton>}
        </span>
      )}
      {onClick && <ChevronRight className="-mr-1 size-icon shrink-0 text-fg-2" strokeWidth={1.5} />}
    </Tag>
  );
}

// Paths use the same UI face and size as setting values; the full path stays in the tooltip.
export function PathText({ path, env, onOpen, className }: { path: string; env: { home: string; cwd: string }; onOpen?: (path: string) => void; className?: string }) {
  const text = shortPath(path, env);
  if (!onOpen) return <span title={path} className={cn('truncate text-2 text-fg-2', className)}>{text}</span>;
  return (
    <button type="button" title={path} onClick={() => onOpen(path)} className={cn('min-w-0 truncate rounded-sm text-2 text-fg-2 underline decoration-line-strong underline-offset-2 transition-colors hover:text-fg-1 hover:decoration-fg-3 focus-visible:text-fg-1', className)}>
      {text}
    </button>
  );
}

// The source is supporting metadata after the items, not another heading level.
// The group's last row: its hover fill runs to the card edges (the card clips the corners) instead of floating inside the inset
export function SourceLink({ path, env, onOpen }: { path: string; env: { home: string; cwd: string }; onOpen: (path: string) => void }) {
  return (
    <button type="button" title={path} onClick={() => onOpen(path)} className={cn(settingRow, '-mx-pad flex items-center gap-gap self-stretch px-pad text-left text-fg-2 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:text-fg-1 focus-visible:bg-hover')}>
      <span className="min-w-0 flex-1 truncate">{shortPath(path, env)}</span>
      <ExternalLink className="size-icon shrink-0" strokeWidth={1.5} />
    </button>
  );
}

// Local text until blur (or Enter on a one-line box), so half-typed values never round-trip through the host
export function TextBox({ value, label, placeholder, multiline, onCommit }: {
  value: string; label: string; placeholder?: string; multiline?: boolean; onCommit: (v: string) => void;
}) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  const commit = () => { if (text !== value) onCommit(text); };
  const cls = 'w-full min-w-0 rounded-md border border-line bg-hover px-3 text-2 text-fg-1 outline-none transition-colors placeholder:text-fg-3 focus:bg-active';
  return multiline
    // Grows with its text; no manual resize handle (it could be dragged to cut a line in half)
    ? <textarea aria-label={label} placeholder={placeholder} value={text} onChange={e => setText(e.target.value)} onBlur={commit}
      className={`${cls} resize-none py-(--setting-row-pad) [field-sizing:content]`} />
    : <input aria-label={label} placeholder={placeholder} value={text} onChange={e => setText(e.target.value)} onBlur={commit}
      onKeyDown={e => { if (e.key === 'Enter') e.currentTarget.blur(); if (e.key === 'Escape') setText(value); }}
      className={`${cls} h-ctl w-(--ctl-w)`} />;
}
