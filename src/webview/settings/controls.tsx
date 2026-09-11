import { createContext, useContext, useEffect, useState, type KeyboardEvent, type ReactNode } from 'react';
import { ChevronRight, ExternalLink } from 'lucide-react';
import { cn } from '../ui/cn';
import { Card } from '../ui/Card';
import { Chip, IconButton } from '../ui/Button';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { Shimmer } from '../ui/Shimmer';
import { Switch as SwitchPrimitive } from '../ui/Switch';
import { t } from '../i18n';

// Settings use a navigation column beside one content column. Page and section headings belong to the content.

export function PageHeader({ title, action }: { title: ReactNode; action?: ReactNode }) {
  return (
    <header className="flex min-h-ctl items-center gap-pad">
      <h1 className="m-0 min-w-0 flex-1 text-(length:--text-h) leading-(--text-h-lh) font-medium text-fg-1 [overflow-wrap:anywhere]">{title}</h1>
      {action && <div className="-mr-1.5 flex shrink-0 items-center">{action}</div>}
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

// Settings rows share typography and vertical padding. Detail rows reserve two text lines; wrapped content can grow.
// Inside a Group (or as a section header) the surface owns the horizontal inset, so the row drops its own.
const GroupContext = createContext(false);
const settingRow = 'min-h-(--setting-row) py-(--setting-row-pad) text-2 data-[detail]:min-h-(--setting-row-detail)';
function useSettingRow() {
  return cn(settingRow, !useContext(GroupContext) && 'px-pad');
}

export function Section({ title, desc, count, action, cards, children }: SectionProps) {
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
    <Card className="acp-section-panel flex flex-col px-pad shadow-none">
      <div className={cn(settingRow, 'flex flex-wrap items-center gap-gap')} data-detail={!!desc || undefined}>
        <div className="min-w-0 flex-[1_1_var(--setting-header-copy)]">
          <h3 className="m-0 flex items-baseline gap-2 text-2 font-normal text-fg-1">
            <span className="[overflow-wrap:anywhere]">{title}</span>
            {count !== undefined && <Count n={count} />}
          </h3>
          {desc && <SectionDescription>{desc}</SectionDescription>}
        </div>
        {action}
      </div>
      {hasContent && <InsetGroupContext.Provider value={true}>
        <div className="border-t border-line">{content}</div>
      </InsetGroupContext.Provider>}
    </Card>
  );
}

// A small text action for a Section's title line: quiet Chip, no caret
export function SectionAction({ icon, onClick, children, title }: { icon?: ReactNode; onClick: () => void; children: ReactNode; title?: string }) {
  return <Chip caret={false} icon={icon} onClick={onClick} title={title} className="-mr-2 shrink-0">{children}</Chip>;
}

export function Group({ className, children }: { className?: string; children: ReactNode }) {
  const inset = useContext(InsetGroupContext);
  const Container = inset ? 'div' : Card;
  return <GroupContext.Provider value={true}>
    <Container className={cn('flex flex-col divide-y divide-line overflow-hidden', !inset && 'px-pad shadow-none', className)}>{children}</Container>
  </GroupContext.Provider>;
}

// One setting: label + description on the left, the control on the right. `stack` puts the control under the text (wide controls)
export function Field({ label, desc, htmlFor, stack, children }: { label?: ReactNode; desc?: ReactNode; htmlFor?: string; stack?: boolean; children?: ReactNode }) {
  return (
    <div className={cn(useSettingRow(), 'flex flex-wrap gap-pad', stack ? 'flex-col' : 'items-center')} data-detail={!!desc || undefined}>
      <div className={cn('min-w-0', stack ? 'w-full' : 'flex-[1_1_var(--setting-header-copy)]')}>
        {label !== undefined && <label htmlFor={htmlFor} className="block text-2 text-fg-1">{label}</label>}
        {desc && <div className="text-2 text-fg-2 [overflow-wrap:anywhere]">{desc}</div>}
      </div>
      {children !== undefined && <div className={cn('flex shrink-0 items-center gap-2', stack && 'w-full min-w-0')}>{children}</div>}
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

export interface Option<V extends string> { value: V; label: string; icon?: ReactNode; hint?: string; disabled?: boolean }

// Two to four exclusive choices as a pill strip; the chosen one is filled, the rest are text until hovered
export function Segmented<V extends string>({ options, value, onChange, label }: { options: Option<V>[]; value: V; onChange: (v: V) => void; label: string }) {
  return (
    <div role="radiogroup" aria-label={label} className="inline-flex max-w-full flex-wrap items-center gap-0.5">
      {options.map(o => (
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          disabled={o.disabled}
          title={o.hint}
          onClick={() => onChange(o.value)}
          className={cn(
            'inline-flex h-ctl items-center gap-1.5 whitespace-nowrap rounded-md px-3 text-2 transition-colors disabled:cursor-not-allowed disabled:opacity-40',
            o.value === value ? 'bg-active text-fg-strong' : 'text-fg-2 hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1',
          )}
        >
          {o.icon && <span className="flex shrink-0 items-center [&_svg]:size-icon">{o.icon}</span>}
          {o.label}
        </button>
      ))}
    </div>
  );
}

// Dropdown select: a bordered Chip that opens the shared Menu; radio semantics, the current value gets the check.
// Sized like NumberField (--ctl tall, --r-md) so the two form controls line up in a column; the label grows so the caret stays at the right edge with or without an icon
export function Select<V extends string>({ options, value, onChange, label, className }: { options: Option<V>[]; value: V; onChange: (v: V) => void; label: string; className?: string }) {
  const cur = options.find(o => o.value === value);
  return <DropdownMenu.Root>
    <DropdownMenu.Trigger render={<Chip aria-label={label} icon={cur?.icon}
      className={cn('h-ctl min-w-(--ctl-w) gap-2 rounded-md border border-line bg-hover px-3 text-2 text-fg-1 hover:bg-active data-[open]:bg-active data-[popup-open]:bg-active', className)}>
      {cur?.label ?? value}
    </Chip>} />
    <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align="end" width="md"><DropdownMenu.Popup>
      <DropdownMenu.RadioGroup value={value} className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
        {options.map(o => <DropdownMenu.RadioItem key={o.value} value={o.value} title={o.hint} disabled={o.disabled} onClick={() => onChange(o.value)}>
          <OptionContent icon={o.icon} checked={o.value === value} checkSlot={!!cur}>{o.label}</OptionContent>
        </DropdownMenu.RadioItem>)}
      </DropdownMenu.RadioGroup>
    </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
  </DropdownMenu.Root>;
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

// Status dot before a status line: ok / off
export function Dot({ ok }: { ok: boolean }) {
  return <span aria-hidden className={cn('inline-block size-(--dot) shrink-0 rounded-full', ok ? 'bg-ok' : 'bg-fg-3')} />;
}

// Filesystem path shortened for display: inside the workspace → relative, under home → ~/…
export function shortPath(path: string, env: { home: string; cwd: string }): string {
  const strip = (root: string) => (root && (path === root || path.startsWith(root.endsWith('/') ? root : `${root}/`)) ? path.slice(root.length).replace(/^\//, '') : undefined);
  const rel = strip(env.cwd);
  if (rel !== undefined) return rel || '.';
  const home = strip(env.home);
  return home !== undefined ? `~/${home}` : path;
}

export interface ItemRowProps {
  lead?: ReactNode;
  title: ReactNode;
  desc?: ReactNode;
  // A line under title / desc spanning the text column (quota bars); not truncated
  extra?: ReactNode;
  trailing?: ReactNode;
  // Hover-revealed open button at the right edge (opens a file)
  onOpen?: () => void;
  // Keep metadata aligned in lists containing both openable and missing files.
  reserveOpen?: boolean;
  // The whole row is the button (navigation): a chevron at the right edge
  onClick?: () => void;
  // Greyed out as a whole (disabled server, missing file)
  dim?: boolean;
  className?: string;
}

export function ItemRow({ lead, title, desc, extra, trailing, onOpen, reserveOpen, onClick, dim, className }: ItemRowProps) {
  const Tag = onClick ? 'button' : 'div';
  return (
    <Tag
      {...(onClick ? { type: 'button' as const, onClick } : {})}
      data-detail={!!desc || undefined}
      className={cn(
        useSettingRow(), 'group/row flex w-full items-center gap-gap text-left',
        onClick && 'transition-colors hover:bg-hover focus-visible:bg-hover',
        className,
      )}
    >
      {lead !== undefined && <span className={cn('flex size-lead shrink-0 items-center justify-center text-fg-2 [&_svg]:size-icon', dim && 'opacity-60')}>{lead}</span>}
      <span className="flex min-w-0 flex-1 flex-col">
        <span className={cn('truncate text-2', dim ? 'text-fg-2' : 'text-fg-1')}>{title}</span>
        {desc && <span className="truncate text-2 text-fg-2">{desc}</span>}
        {extra}
      </span>
      {trailing && <span className="flex shrink-0 items-center gap-2">{trailing}</span>}
      {(onOpen || reserveOpen) && (
        <span className="-mr-1.5 flex size-ctl shrink-0 items-center justify-center" aria-hidden={!onOpen || undefined}>
          {onOpen && <IconButton title={t('common.open')} aria-label={t('common.open')} onClick={onOpen} className="text-fg-2 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100">
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
export function SourceLink({ path, env, onOpen }: { path: string; env: { home: string; cwd: string }; onOpen: (path: string) => void }) {
  return (
    <button type="button" title={path} onClick={() => onOpen(path)} className={cn(useSettingRow(), 'flex w-full items-center gap-gap rounded-md text-left text-fg-2 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:text-fg-1 focus-visible:bg-hover')}>
      <span className="min-w-0 flex-1 truncate">{shortPath(path, env)}</span>
      <ExternalLink className="size-icon shrink-0" strokeWidth={1.5} />
    </button>
  );
}
