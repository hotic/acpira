import { createContext, useContext, useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type MouseEvent, type ReactNode } from 'react';
import { ArrowUpRight, Check, ChevronRight, ChevronsUpDown, Search, Trash2, X } from 'lucide-react';
import { Radio } from '@base-ui/react/radio';
import { RadioGroup } from '@base-ui/react/radio-group';
import { cn } from '../ui/cn';
import { Card } from '../ui/Card';
import { IconButton } from '../ui/Button';
import { Dialog } from '../ui/Dialog';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { Shimmer } from '../ui/Shimmer';
import { StableText } from '../ui/StableText';
import { Switch as SwitchPrimitive } from '../ui/Switch';
import { t } from '../i18n';

// Settings use a navigation column beside one content column. Page and section headings belong to the content.
// The system (settled 2026-10-26, lab/settings-all.preview.tsx): card heads carry no icon (title, count, one fg-3 line,
// one quiet action); rows sit on the text edge, with an icon column only when every row of the list has its own icon;
// three text roles (titles fg-1, secondary fg-3, actions fg-2); no chevrons, a clickable row lights up instead; every
// control of the right column shares one width; whatever a row opens opens in a dialog, whose groups are filled blocks

// The page title: larger and heavier than a card title, so the page reads as one level above its cards
export function PageHeader({ title, action }: { title: ReactNode; action?: ReactNode }) {
  return (
    <header className="flex min-h-ctl items-center gap-pad pb-1">
      <h1 className="m-0 min-w-0 flex-1 text-(length:--text-page) leading-(--text-page-lh) font-semibold text-fg-1 [overflow-wrap:anywhere]">{title}</h1>
      {action && <div className="flex shrink-0 items-center">{action}</div>}
    </header>
  );
}

// A drilled-in page's title: the page it came from, a chevron, then its own name. The parent is the way back up (a step
// on the settings' back / forward trail like any link), so the page needs no back button of its own
export function PageCrumb({ parent, onParent, children }: { parent: string; onParent: () => void; children: ReactNode }) {
  return (
    <span className="flex min-w-0 items-center gap-1">
      <button type="button" onClick={onParent} className="shrink-0 rounded-sm text-fg-2 transition-colors hover:text-fg-1 focus-visible:text-fg-1">{parent}</button>
      <ChevronRight className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} aria-hidden />
      <span className="min-w-0 truncate">{children}</span>
    </span>
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
  return <span className={cn('shrink-0 text-2 font-normal text-fg-3 tabular-nums', className)}>{n}</span>;
}

export interface SectionProps {
  title?: ReactNode;
  desc?: ReactNode;
  count?: number;
  // A small status mark between the title and the count (a Dot with its label)
  status?: ReactNode;
  // One small action (a SectionAction), at the right of the title line
  action?: ReactNode;
  // Children are already Cards (several groups), not rows to be wrapped in one Group
  cards?: boolean;
  children?: ReactNode;
}

// Copy outside a card (a page's footnote): the secondary role, on the card's outer edge
export function SectionDescription({ children }: { children: ReactNode }) {
  return <p className="m-0 text-3 text-fg-3 [overflow-wrap:anywhere]">{children}</p>;
}

// Titled subgroups own one surface. Their lists inherit it and use separators instead of nested cards.
const InsetGroupContext = createContext(false);
// Inside a SettingsDialog every card becomes a filled block (bg-chip, rounded, no border), like the tab strip and the
// search box above it: bordered cards in a dialog read as boxes stacked on boxes. Its inner padding is the search box's,
// so the search glass, the rows' icons and their titles share one line
const DialogContext = createContext(false);
export const useInDialog = () => useContext(DialogContext);
const block = 'flex flex-col overflow-hidden rounded-lg bg-chip px-3';

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
// A settings row's padding and leading: its own side padding only outside a Group (whose block pads it)
export function useSettingRow() {
  return cn(settingRow, !useContext(GroupContext) && 'px-pad');
}
// A title over its second line: the two lines a few pixels apart, touching they read as crowded
const stack = 'flex min-w-0 flex-col gap-y-(--stack-gap)';
// The secondary role: every second line, count and caption
const second = 'text-3 text-fg-3';

// A card (a block inside a dialog) holding the given content
function Surface({ className, children }: { className?: string; children: ReactNode }) {
  if (useContext(DialogContext)) return <section className={cn(block, className)}>{children}</section>;
  return <Card className={cn('acp-section-panel flex flex-col overflow-hidden px-pad shadow-none', className)}>{children}</Card>;
}

export function Section({ title, desc, count, status, action, cards, children }: SectionProps) {
  const inset = title !== undefined || !!action;
  const hasContent = children !== null && children !== undefined && children !== false;
  const content = cards ? <div className={cn('flex flex-col', !inset && 'gap-pad')}>{children}</div> : <Group>{children}</Group>;
  if (!inset) return (
    <div className="flex flex-col gap-2">
      {desc && <SectionDescription>{desc}</SectionDescription>}
      {content}
    </div>
  );
  return (
    <Surface>
      <ListHead title={title} desc={desc} count={count} status={status} action={action} />
      {hasContent && <InsetGroupContext.Provider value={true}>
        <div className={topRule}>{content}</div>
      </InsetGroupContext.Provider>}
    </Surface>
  );
}

// The head of a titled list: the title with its status and count, an optional second line, and the action at the right.
// A Section card's head, and a group head inside a shared card. No icon: a head mark left the column under it empty
// on every setting row (studied 2026-10-26)
export function ListHead({ title, desc, count, status, action }: Pick<SectionProps, 'title' | 'desc' | 'count' | 'status' | 'action'>) {
  return (
    <div className={cn(settingRow, 'flex flex-wrap items-center gap-gap')} data-detail={!!desc || undefined}>
      <div className={cn(stack, 'flex-[1_1_var(--setting-header-copy)]')}>
        <h3 className="m-0 flex items-baseline gap-2 text-2 font-medium text-fg-1">
          <span className="min-w-0 [overflow-wrap:anywhere]">{title}</span>
          {status}
          {count !== undefined && <Count n={count} />}
        </h3>
        {desc && <div className={cn(second, '[overflow-wrap:anywhere]')}>{desc}</div>}
      </div>
      {action}
    </div>
  );
}

// The one search field: a box whose glass sits in a slot as wide as a row's icon column, so inside a dialog the glass,
// the rows' icons and their titles line up. `className` sizes it in its row; `children` sit at its right end
export function SearchBox({ value, onChange, label, className, autoFocus }: { value: string; onChange: (v: string) => void; label: string; className?: string; autoFocus?: boolean }) {
  return (
    <div className={cn('flex h-ctl min-w-0 items-center gap-gap rounded-md border border-line bg-chip px-3 focus-within:border-line-strong', className)}>
      <span className="flex w-lead shrink-0 items-center justify-center"><Search className="size-icon text-fg-3" strokeWidth={1.75} aria-hidden /></span>
      <input type="search" value={value} onChange={e => onChange(e.target.value)} aria-label={label} placeholder={label} autoFocus={autoFocus}
        className="min-w-0 flex-1 border-0 bg-transparent text-2 text-fg-1 outline-none placeholder:text-fg-3" />
    </div>
  );
}

// A search field with an optional control at its right (a list's master switch, an add button)
export function SearchRow({ value, onChange, label, children }: { value: string; onChange: (v: string) => void; label?: string; children?: ReactNode }) {
  return (
    <div className="flex min-w-0 items-center gap-gap">
      <SearchBox value={value} onChange={onChange} label={label ?? t('settings.models.search')} className="flex-1" />
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  );
}

// A card head's one action: a quiet icon + word. Its own padding hangs past the cards' right edge, so its words end on
// the rows' text edge
export function SectionAction({ icon, onClick, children, title, disabled }: { icon?: ReactNode; onClick: () => void; children: ReactNode; title?: string; disabled?: boolean }) {
  return (
    <button type="button" onClick={onClick} title={title} disabled={disabled}
      className="-mr-2 flex h-ctl-sm shrink-0 items-center gap-1 rounded-md px-2 text-2 whitespace-nowrap text-fg-2 transition-colors enabled:hover:bg-hover enabled:hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1 disabled:opacity-50 [&_svg]:size-icon [&_svg]:stroke-[1.75]">
      {icon}{children}
    </button>
  );
}

export function Group({ className, children, embedded = false }: { className?: string; children: ReactNode; embedded?: boolean }) {
  const inheritedInset = useContext(InsetGroupContext);
  const dialog = useContext(DialogContext);
  const inset = embedded || inheritedInset;
  const list = cn('flex flex-col', rowRules, className);
  return <GroupContext.Provider value={true}>
    {inset ? <div className={list}>{children}</div>
      : dialog ? <section className={cn(block, rowRules, className)}>{children}</section>
      : <Card className={cn(list, 'overflow-hidden px-pad shadow-none')}>{children}</Card>}
  </GroupContext.Provider>;
}

// A segmented tab strip (a dialog's tabs, the Shared page's lists): one filled track, the current tab lit
export function Tabs<V extends string>({ items, value, onChange, label }: {
  items: { value: V; label: string; count?: number }[];
  value: V;
  onChange: (v: V) => void;
  label: string;
}) {
  const move = (e: KeyboardEvent<HTMLButtonElement>, i: number) => {
    const step = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0;
    if (!step) return;
    e.preventDefault();
    const next = items[(i + step + items.length) % items.length]!;
    onChange(next.value);
    (e.currentTarget.parentElement?.children[items.indexOf(next)] as HTMLElement | undefined)?.focus();
  };
  return (
    <div role="tablist" aria-label={label} className="flex h-ctl shrink-0 [scrollbar-width:none] items-center gap-0.5 overflow-x-auto rounded-lg bg-chip p-0.5">
      {items.map((it, i) => {
        const on = it.value === value;
        return (
          <button key={it.value} type="button" role="tab" aria-selected={on} tabIndex={on ? 0 : -1} onClick={() => onChange(it.value)} onKeyDown={e => move(e, i)}
            className={cn('flex h-full min-w-max flex-1 items-center justify-center gap-1.5 rounded-md px-2 text-2 whitespace-nowrap outline-none transition-colors focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-fg-2',
              on ? 'bg-active text-fg-1' : 'text-fg-2 hover:text-fg-1')}>
            {it.label}
            {it.count !== undefined && <span className="text-fg-3 tabular-nums">{it.count}</span>}
          </button>
        );
      })}
    </div>
  );
}

// A segmented single choice (a radio group drawn like the tab strip): `w-(--ctl-w)` as a row's control, or a line of
// its own
export function Segmented<V extends string>({ items, value, onChange, label, className }: {
  items: { value: V; label: ReactNode }[];
  value: V;
  onChange: (v: V) => void;
  label: string;
  className?: string;
}) {
  return (
    <RadioGroup value={value} onValueChange={v => onChange(v as V)} aria-label={label}
      className={cn('flex h-ctl min-w-0 shrink-0 items-center gap-0.5 rounded-lg bg-chip p-0.5', className)}>
      {items.map(it => (
        <Radio.Root key={it.value} value={it.value} render={<button type="button" />} nativeButton
          onKeyDownCapture={e => { if (e.key === 'Enter') { e.preventDefault(); e.currentTarget.click(); } }}
          className={cn('inline-flex h-full min-w-max flex-1 items-center justify-center gap-1.5 rounded-md px-2 text-2 whitespace-nowrap outline-none transition-colors focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-fg-2 [&_svg]:size-icon',
            it.value === value ? 'bg-active text-fg-1' : 'text-fg-2 hover:text-fg-1')}>
          {it.label}
        </Radio.Root>
      ))}
    </RadioGroup>
  );
}

// The settings pages' dialog frame: a scrim over the settings shell, a head (an optional mark, the title with a fg-3
// subtitle, an optional control such as the item's switch, close), the body, and an optional footer. A click on the
// scrim closes. Inside, Sections and Groups render as filled blocks. `tall` gives a tabbed dialog one fixed height, so
// switching tabs never resizes it. It shares the menus' z-30 layer: a select inside portals its list after it into the
// same layer and so paints above it; a dialog opened from a dialog paints above the one it came from
export function SettingsDialog({ title, sub, icon, control, wide, tall, onClose, footer, children }: {
  title: string;
  sub?: ReactNode;
  icon?: ReactNode;
  control?: ReactNode;
  wide?: boolean;
  tall?: boolean;
  onClose: () => void;
  footer?: ReactNode;
  children: ReactNode;
}) {
  const scrim = (e: MouseEvent) => { if (e.target === e.currentTarget) onClose(); };
  return <Dialog.Root open onOpenChange={next => { if (!next) onClose(); }}>
    <Dialog.Portal>
      {/* acp-settings: the settings measures (dialog widths, row leading) also where the dialog opens outside the
          settings shell, such as the remote folder picker from the session column */}
      <Dialog.Popup onClick={scrim} className="acp-settings absolute inset-0 z-30 flex items-center justify-center bg-scrim p-pad">
        <div className={cn('flex max-h-full max-w-full flex-col overflow-hidden rounded-lg border border-line bg-bg-1 shadow-pop', wide ? 'w-(--dialog-w-wide)' : 'w-(--dialog-w)', tall && 'h-(--dialog-h)')}>
          <header className="flex shrink-0 items-center gap-gap border-b border-line py-gap pr-gap pl-pad">
            {icon && <span className="flex size-lead shrink-0 items-center justify-center text-fg-2 [&>svg]:size-lead [&>svg]:stroke-[1.5]">{icon}</span>}
            <span className="flex min-w-0 flex-1 items-baseline gap-2">
              {/* The title keeps its words; the subtitle gives way first */}
              <Dialog.Title className="m-0 min-w-0 max-w-full shrink-0 truncate text-2 font-medium text-fg-1">{title}</Dialog.Title>
              {sub && <span className="min-w-0 truncate text-2 text-fg-3">{sub}</span>}
            </span>
            {control}
            <IconButton size="sm" title={t('common.close')} aria-label={t('common.close')} onClick={onClose} className="text-fg-2"><X strokeWidth={1.5} /></IconButton>
          </header>
          <DialogContext.Provider value={true}>{children}</DialogContext.Provider>
          {footer && <div className="flex shrink-0 items-center gap-2 border-t border-line px-pad py-gap">{footer}</div>}
        </div>
      </Dialog.Popup>
    </Dialog.Portal>
  </Dialog.Root>;
}

// A dialog's content column: tab strip, search box and blocks stacked on one outer edge, scrolling as one
export function DialogBody({ children, className }: { children: ReactNode; className?: string }) {
  // Blocks keep their height and the body scrolls; a shrinking block would clip its own rows instead.
  // The thin gutter stays reserved (scroll-stable), so a filter that crosses the dialog height never shifts the
  // column sideways; the right padding gives that gutter back, keeping both text edges at pad. A body marked
  // overflow-hidden holds its own scroller and gets no gutter (the unlayered scroll-stable would beat overflow-hidden)
  const scrolls = !/(^|\s)overflow-hidden(\s|$)/.test(className ?? '');
  return <div className={cn('scroll-thin flex min-h-0 flex-1 flex-col gap-gap overflow-y-auto p-pad [&>*]:shrink-0', scrolls && 'scroll-stable pr-dialog-gutter', className)}>{children}</div>;
}

// A dialog footer's destructive action, on the left: quiet until hovered, its words on the body's text edge
export function DangerAction({ onClick, children, disabled, title, icon = <Trash2 strokeWidth={1.75} /> }: { onClick: () => void; children: ReactNode; disabled?: boolean; title?: string; icon?: ReactNode }) {
  return (
    <button type="button" onClick={onClick} disabled={disabled} title={title}
      className="-ml-2 flex h-ctl shrink-0 items-center gap-1.5 rounded-md px-2 text-2 text-fg-2 transition-colors enabled:hover:bg-hover enabled:hover:text-danger focus-visible:bg-hover focus-visible:text-danger disabled:opacity-50 [&_svg]:size-icon">
      {icon}{children}
    </button>
  );
}

// A footer's note (where a file lives, what is picked): the secondary role, taking the room between the actions
export function FooterNote({ children, className }: { children: ReactNode; className?: string }) {
  return <span className={cn('min-w-0 flex-1 truncate text-3 text-fg-3', className)}>{children}</span>;
}

// A dialog form's field: its label above the control (the source form, the model editor); `hint` is the label's tooltip
export function FormField({ label, hint, children, className }: { label: string; hint?: string; children: ReactNode; className?: string }) {
  return (
    <div className={cn('flex min-w-0 flex-col gap-1', className)}>
      <span className="text-3 text-fg-3" title={hint}>{label}</span>
      {children}
    </div>
  );
}

// A dialog form: fields two to a line where the dialog is wide enough, one where it is not; `FormField className="col-span-full"`
// takes a whole line
export function FormGrid({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn('grid grid-cols-[repeat(auto-fit,minmax(var(--setting-header-copy),1fr))] gap-3 p-pad', className)}>{children}</div>;
}

// Setting rows under a dialog's form, bare (no block): they start on the form labels' edge, a rule above the first
export function FormRows({ children }: { children: ReactNode }) {
  return <GroupContext.Provider value={true}><div className={cn('relative flex flex-col px-pad before:pointer-events-none before:absolute before:top-0 before:inset-x-0 before:h-px before:bg-line', rowRules)}>{children}</div></GroupContext.Provider>;
}

// One setting: label + description on the left, the control on the right. `stack` puts the control under the text (wide controls)
export function Field({ label, desc, htmlFor, stack: stacked, children }: { label?: ReactNode; desc?: ReactNode; htmlFor?: string; stack?: boolean; children?: ReactNode }) {
  return (
    <div className={cn(useSettingRow(), 'flex flex-wrap gap-x-pad gap-y-2', stacked ? 'flex-col' : 'items-center')} data-detail={!!desc || undefined}>
      <div className={cn(stack, stacked ? 'w-full' : 'flex-[1_1_var(--setting-header-copy)]')}>
        {label !== undefined && <label htmlFor={htmlFor} className="block text-2 text-fg-1">{label}</label>}
        {desc && <div className={cn(second, '[overflow-wrap:anywhere]')}>{desc}</div>}
      </div>
      {/* max-w-full: once the row wraps, a long Select label truncates inside the card instead of overflowing it */}
      {children !== undefined && <div className={cn('flex max-w-full shrink-0 items-center gap-2', stacked && 'w-full min-w-0')}>{children}</div>}
    </div>
  );
}

// A fact: its name as the row's title, the value as its second line
export function FactRow({ label, children, title }: { label: ReactNode; children: ReactNode; title?: string }) {
  return (
    <div className={cn(useSettingRow(), stack, 'justify-center')} data-detail>
      <span className="text-2 text-fg-1">{label}</span>
      <span className={cn(second, 'flex min-w-0 items-center gap-1.5')} title={title}>{children}</span>
    </div>
  );
}

// A faint single-line row inside a Group (empty states, notes)
export function Note({ children, shimmer }: { children: ReactNode; shimmer?: boolean }) {
  return <div className={cn(useSettingRow(), 'flex items-center text-fg-3')}><Shimmer active={!!shimmer}>{children}</Shimmer></div>;
}

// description: a caption under the label in the list only (the field shows the label alone)
export interface Option<V extends string> { value: V; label: string; icon?: ReactNode; hint?: string; disabled?: boolean; description?: string }

// Dropdown select: a bordered field that opens the shared Menu; radio semantics, the current value gets the check.
// Every control of the right column shares one width (--ctl-w), so a column of selects, number fields and text boxes
// lines up; a label longer than that truncates, its list row keeps it whole. A className width (w-full in a form
// dialog) wins; the list follows it. The list is painted in the field's own material (`fieldTone`), and the field
// keeps its resting fill while open, so the two read as one control instead of a lighter field over a darker menu
export function Select<V extends string>({ options, value, onChange, label, className }: { options: Option<V>[]; value: V; onChange: (v: V) => void; label: string; className?: string }) {
  const cur = options.find(o => o.value === value);
  // Rows with a caption get the model panels' two-line padding, all of them so the list keeps one rhythm
  const twoLine = options.some(o => o.description);
  const trigger = useRef<HTMLButtonElement>(null);
  const [tone, setTone] = useState<CSSProperties>();
  return <DropdownMenu.Root onOpenChange={open => { if (open) setTone(fieldTone(trigger.current)); }}>
    <DropdownMenu.Trigger render={<button ref={trigger} type="button" aria-label={label}
      className={cn('flex h-ctl w-(--ctl-w) max-w-full min-w-0 items-center gap-2 rounded-md border border-line bg-hover px-3 text-left text-2 text-fg-1 transition-colors hover:border-line-strong hover:bg-active data-[popup-open]:border-line-strong data-[popup-open]:bg-hover', className)}>
      {cur?.icon && <span className="flex shrink-0 items-center [&_svg]:size-icon">{cur.icon}</span>}
      {/* Every label laid out in one cell, only the current one visible: a pick never moves the caret */}
      <StableText className="min-w-0 flex-1" itemClassName="truncate" current={value}
        items={[...options.map(o => ({ key: o.value, text: o.label })), ...(cur ? [] : [{ key: value, text: value }])]} />
      <ChevronsUpDown className="size-icon shrink-0 text-fg-3" strokeWidth={1.75} aria-hidden />
    </button>} />
    <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align="start" width="anchor"><DropdownMenu.Popup style={tone}>
      <DropdownMenu.RadioGroup value={value} className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
        {options.map(o => <DropdownMenu.RadioItem key={o.value} value={o.value} title={o.hint ?? o.label} disabled={o.disabled} onClick={() => onChange(o.value)} className={twoLine ? 'min-h-0 py-1.5' : undefined}>
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

// Numeric input committing on blur / Enter: a plain text field at the control width, the unit at its trailing edge.
// Local text state so half-typed values don't round-trip through the host; out-of-range input snaps back to the
// current value rather than being clamped, so a typo never silently lands on a bound; ArrowUp / ArrowDown step within bounds
export function NumberField({ value, onCommit, min, max, step = 1, unit, label }: { value: number; onCommit: (v: number) => void; min?: number; max?: number; step?: number; unit?: string; label: string }) {
  const [text, setText] = useState(String(value));
  useEffect(() => { setText(String(value)); }, [value]);
  const commit = () => {
    const n = Number(text);
    if (!text.trim() || !Number.isFinite(n) || (min !== undefined && n < min) || (max !== undefined && n > max)) { setText(String(value)); return; }
    setText(String(n));
    if (n !== value) onCommit(n);
  };
  const bump = (dir: 1 | -1) => {
    const base = Number(text);
    let n = (Number.isFinite(base) ? base : value) + dir * step;
    if (min !== undefined) n = Math.max(min, n);
    if (max !== undefined) n = Math.min(max, n);
    setText(String(n));
    if (n !== value) onCommit(n);
  };
  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') e.currentTarget.blur();
    else if (e.key === 'Escape') setText(String(value));
    else if (e.key === 'ArrowUp') { e.preventDefault(); bump(1); }
    else if (e.key === 'ArrowDown') { e.preventDefault(); bump(-1); }
  };
  return (
    <label className="flex h-ctl w-(--ctl-w) max-w-full cursor-text items-center gap-1.5 rounded-md border border-line bg-hover px-3 text-2 text-fg-1 tabular-nums transition-colors hover:border-line-strong focus-within:border-line-strong focus-within:bg-active">
      <input
        type="text"
        inputMode="numeric"
        aria-label={label}
        value={text}
        onChange={e => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={onKey}
        className="min-w-0 flex-1 bg-transparent outline-none"
      />
      {unit && <span className="shrink-0 select-none text-fg-3">{unit}</span>}
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

// A dot and its words, as a row's status value
export function Status({ ok, warn, children }: { ok?: boolean; warn?: boolean; children: ReactNode }) {
  return <span className="flex min-w-0 items-center gap-1.5 text-2 text-fg-2"><Dot ok={!!ok} warn={warn} /><span className="truncate">{children}</span></span>;
}

// A quiet value at a row's right edge (a size, a state)
export function Meta({ children, title }: { children: ReactNode; title?: string }) {
  return <span className="shrink-0 text-2 text-fg-3 tabular-nums" title={title}>{children}</span>;
}

// A Command list (the add-model and add-host pickers) drawn as a dialog block: the list is the block and its items are
// setting rows, the highlight running from edge to edge. Its rules stay inside the list: one past its sides would make
// the scrolling list scroll sideways
export const blockList = 'rounded-lg bg-chip [&>*+*]:relative [&>*+*]:after:pointer-events-none [&>*+*]:after:absolute [&>*+*]:after:top-0 [&>*+*]:after:inset-x-0 [&>*+*]:after:h-px [&>*+*]:after:bg-line';
export const blockItem = 'min-h-(--setting-row-detail) gap-gap rounded-none px-3 py-(--setting-row-pad)';
export function PickContent({ lead, title, desc, checked }: { lead?: ReactNode; title: ReactNode; desc?: ReactNode; checked?: boolean }) {
  return <>
    {lead && <span className="flex size-lead shrink-0 items-center justify-center text-fg-2 [&>svg]:size-lead [&>svg]:stroke-[1.5]">{lead}</span>}
    <span className={cn(stack, 'flex-1')}>
      <span className="truncate text-2 text-fg-1">{title}</span>
      {desc && <span className={cn(second, 'truncate')}>{desc}</span>}
    </span>
    <span className="flex size-icon shrink-0 items-center justify-center">{checked && <Check className="size-icon text-fg-1" strokeWidth={2} />}</span>
  </>;
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
  // The row opens something outside the settings (a file, a folder, a page): a click anywhere on it but its own controls
  // opens, and a ↗ glyph stays at the right edge as the cue and the keyboard stop
  onOpen?: () => void;
  // Keep metadata aligned in lists containing both openable and missing files.
  reserveOpen?: boolean;
  // The row opens its dialog: a click anywhere on it but its own controls; the row lights up, no glyph
  onEdit?: () => void;
  editLabel?: string;
  // The whole row is the button
  onClick?: () => void;
  // Greyed out as a whole (disabled server, missing file)
  dim?: boolean;
  className?: string;
}

// Full-width hover fill for a clickable row inside an inset Group: the row keeps the group's width (its text stays on the
// inset), while `::before` paints (and takes pointer hits) from card edge to card edge, between the rules above and below.
// One rule for every card: a row lights up only when a click anywhere on it does something (a dialog, open, a row-shaped
// text button); rows that merely hold controls or information stay still
export const rowHover = 'relative isolate before:absolute before:inset-y-0 before:-inset-x-pad before:-z-10 before:transition-colors enabled:hover:before:bg-hover focus-visible:before:bg-hover';
// The same fill for a row that is a div (it may hold buttons of its own)
const divHover = 'relative isolate cursor-pointer before:absolute before:inset-y-0 before:-inset-x-pad before:-z-10 before:transition-colors hover:before:bg-hover has-[[data-row-key]:focus-visible]:before:bg-hover';
const OWN_CONTROL = 'button:not([data-row-key]), a, input, textarea, select, label, [role="switch"], [role="checkbox"], [role="menu"]';

export function ItemRow({ lead, title, desc, extra, trailing, onOpen, reserveOpen, onEdit, editLabel, onClick, dim, className }: ItemRowProps) {
  const Tag = onClick ? 'button' : 'div';
  const press = !onClick ? onEdit ?? onOpen : undefined;
  // A click on one of the row's own controls (a switch, remove, …) stays theirs; anywhere else on the row opens
  const pressFrom = press ? (e: MouseEvent<HTMLElement>) => {
    const hit = (e.target as Element).closest(OWN_CONTROL);
    if (!hit || !e.currentTarget.contains(hit)) press();
  } : undefined;
  return (
    <Tag
      {...(onClick ? { type: 'button' as const, onClick } : { onClick: pressFrom })}
      data-detail={!!desc || undefined}
      className={cn(
        useSettingRow(), 'group/row flex w-full items-center gap-gap text-left',
        onClick && rowHover,
        press && divHover,
        className,
      )}
    >
      {/* The row's own key stop when the row opens a dialog: a zero-size button over the title */}
      {onEdit && !onClick && <button type="button" data-row-key onClick={onEdit} aria-label={editLabel} aria-haspopup="dialog" className="sr-only focus-visible:not-sr-only focus-visible:absolute focus-visible:inset-0 focus-visible:outline-none" />}
      {lead !== undefined && <span className={cn('flex size-lead shrink-0 items-center justify-center text-fg-2 [&>svg]:size-lead [&>svg]:stroke-[1.5]', dim && 'opacity-60')}>{lead}</span>}
      <span className={cn(stack, 'flex-1')}>
        <span className={cn('truncate text-2', dim ? 'text-fg-3' : 'text-fg-1')}>{title}</span>
        {desc && <span className={cn(second, 'truncate')}>{desc}</span>}
        {extra}
      </span>
      {/* At most half the row, so a long marker (a skill's conflict) truncates instead of squeezing the title away */}
      {trailing && <span className="relative flex min-w-0 max-w-1/2 shrink-0 items-center gap-2">{trailing}</span>}
      {(onOpen || reserveOpen) && (
        // Zero height: the button overflows the slot evenly, so reserving it never makes the row taller than its neighbours
        <span className="-mr-1.5 flex h-0 w-ctl shrink-0 items-center justify-center" aria-hidden={!onOpen || undefined}>
          {/* Always shown as the row's cue; the pointer goes through to the row, the keyboard still stops here */}
          {onOpen && <IconButton title={t('common.open')} aria-label={t('common.open')} onClick={onOpen}
            className={!onEdit ? 'pointer-events-none text-fg-3 group-hover/row:text-fg-2' : 'text-fg-3 hover:text-fg-1'}>
            <ArrowUpRight strokeWidth={1.75} />
          </IconButton>}
        </span>
      )}
    </Tag>
  );
}

// Paths take the role of the line they sit in; the full path stays in the tooltip
export function PathText({ path, env, onOpen, className }: { path: string; env: { home: string; cwd: string }; onOpen?: (path: string) => void; className?: string }) {
  const text = shortPath(path, env);
  if (!onOpen) return <span title={path} className={cn('truncate', className)}>{text}</span>;
  return (
    <button type="button" title={path} onClick={() => onOpen(path)} className={cn('min-w-0 truncate rounded-sm underline decoration-line-strong underline-offset-2 transition-colors hover:text-fg-1 hover:decoration-fg-3 focus-visible:text-fg-1', className)}>
      {text}
    </button>
  );
}

// The source is supporting metadata after the items, not another heading level.
// The group's last row: its hover fill runs to the card edges (the card clips the corners) instead of floating inside the inset
export function SourceLink({ path, env, onOpen }: { path: string; env: { home: string; cwd: string }; onOpen: (path: string) => void }) {
  return (
    <button type="button" title={path} onClick={() => onOpen(path)} className={cn(settingRow, '-mx-pad flex items-center gap-gap self-stretch px-pad text-left text-fg-3 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:text-fg-1 focus-visible:bg-hover')}>
      <span className="min-w-0 flex-1 truncate">{shortPath(path, env)}</span>
      <ArrowUpRight className="size-icon shrink-0" strokeWidth={1.75} />
    </button>
  );
}

// Local text until blur (or Enter on a one-line box), so half-typed values never round-trip through the host
export function TextBox({ value, label, placeholder, multiline, onCommit, className }: {
  value: string; label: string; placeholder?: string; multiline?: boolean; onCommit: (v: string) => void; className?: string;
}) {
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  const commit = () => { if (text !== value) onCommit(text); };
  const cls = 'w-full min-w-0 rounded-md border border-line bg-hover px-3 text-2 text-fg-1 outline-none transition-colors placeholder:text-fg-3 hover:border-line-strong focus:bg-active';
  return multiline
    // Grows with its text; no manual resize handle (it could be dragged to cut a line in half)
    ? <textarea aria-label={label} placeholder={placeholder} value={text} onChange={e => setText(e.target.value)} onBlur={commit}
      className={cn(cls, 'min-h-(--setting-row-detail) resize-none py-2 [field-sizing:content]', className)} />
    : <input aria-label={label} placeholder={placeholder} value={text} onChange={e => setText(e.target.value)} onBlur={commit}
      onKeyDown={e => { if (e.key === 'Enter') e.currentTarget.blur(); if (e.key === 'Escape') setText(value); }}
      className={cn(cls, 'h-ctl w-(--ctl-w)', className)} />;
}
