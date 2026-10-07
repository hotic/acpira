import type { ReactNode } from 'react';
import { ChevronDown } from 'lucide-react';
import { Switch } from './Switch';
import { Radio } from '@base-ui/react/radio';
import { RadioGroup } from '@base-ui/react/radio-group';
import { DropdownMenu } from './DropdownMenu';
import { OptionContent } from './Panel';
import { cn } from './cn';

// Neutral form controls for menu footers: switch rows, segmented single-select groups, ordinal segment scales, header toggles and inline dropdown rows.

export interface SwitchRowProps {
  label: ReactNode;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}

// A --row-tall row with the label on the left and the switch at the end; the whole row is the hit target
export function SwitchRow({ label, checked, disabled, onChange }: SwitchRowProps) {
  return (
    <Switch skin="menu"
      checked={checked}
      disabled={disabled}
      onCheckedChange={onChange}
      className="flex min-h-row w-full items-center gap-2 rounded-md px-2 text-left text-2 text-fg-1 outline-none transition-colors hover:bg-hover focus-visible:bg-hover disabled:text-fg-3 disabled:hover:bg-transparent"
    >
      <span className="min-w-0 flex-1 truncate">{label}</span>
      <span className={cn('relative flex h-switch-track-h w-switch-track-w shrink-0 items-center rounded-full transition-colors', checked ? 'bg-btn-1' : 'bg-active', disabled && 'opacity-50')}>
        <span className={cn('size-switch-thumb rounded-full transition-transform', checked ? 'translate-x-switch-on bg-btn-1-fg' : 'translate-x-switch-off bg-fg-3')} />
      </span>
    </Switch>
  );
}

export interface SelectRowProps<V extends string> {
  label: string;
  options: { value: V; label: string; disabled?: boolean }[];
  value: V;
  onChange: (value: V) => void;
}

// A --row-tall row with the label on the left and the current value + caret at the end; the whole row opens a radio menu.
// For dimensions with too many values to lay out as pills (a Fusion lead / sidekick); disabled entries stay listed so the menu keeps its shape
export function SelectRow<V extends string>({ label, options, value, onChange }: SelectRowProps<V>) {
  const cur = options.find(o => o.value === value);
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger aria-label={label}
        className="flex min-h-row w-full items-center gap-2 rounded-md px-2 text-left text-2 text-fg-1 outline-none transition-colors hover:bg-hover focus-visible:bg-hover data-[popup-open]:bg-hover">
        <span className="min-w-0 flex-1 truncate">{label}</span>
        <span className="min-w-0 truncate text-fg-2">{cur?.label ?? value}</span>
        <ChevronDown className="size-3 shrink-0 text-fg-3" strokeWidth={1.75} />
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align="end" width="sm"><DropdownMenu.Popup>
        <DropdownMenu.RadioGroup value={value} className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
          {options.map(o => <DropdownMenu.RadioItem key={o.value} value={o.value} disabled={o.disabled} onClick={() => onChange(o.value)}>
            <OptionContent checked={o.value === value} checkSlot={!!cur}>{o.label}</OptionContent>
          </DropdownMenu.RadioItem>)}
        </DropdownMenu.RadioGroup>
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

export interface RadioPillsProps<V extends string> {
  label: string;
  options: { value: V; label: string; disabled?: boolean }[];
  value: V;
  onChange: (value: V) => void;
}

// Content-sized segments share spare space without squeezing longer labels; narrow tracks can wrap.
export function RadioPills<V extends string>({ label, options, value, onChange }: RadioPillsProps<V>) {
  return (
    <RadioGroup value={value} onValueChange={v => onChange(v as V)} aria-label={label} className="flex min-w-0 flex-1 flex-wrap gap-0.5 rounded-md bg-hover p-0.5">
      {options.map(o => (
        <Radio.Root render={<button type="button" />} nativeButton
          key={o.value}
          value={o.value}
          disabled={o.disabled}
          onKeyDownCapture={event => {
            // Preserve the former button's Enter activation alongside Space.
            if (event.key === 'Enter') {
              event.preventDefault();
              event.currentTarget.click();
            }
          }}
          className={cn(
            'inline-flex h-ctl-sm min-w-max grow basis-auto items-center justify-center whitespace-nowrap rounded-sm px-1.5 text-3 outline-none transition-colors focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-fg-2 disabled:text-fg-3 disabled:opacity-50',
            o.value === value ? 'bg-active text-fg-1' : 'text-fg-2 enabled:hover:bg-hover enabled:hover:text-fg-1',
          )}
        >
          {o.label}
        </Radio.Root>
      ))}
    </RadioGroup>
  );
}

export interface SegmentScaleProps<V extends string> {
  label: string;
  // `ultra` marks the overdrive tier past Max (Codex's `ultra` effort): it closes the row in the ultra tint, text only
  options: { value: V; label: string; disabled?: boolean; ultra?: boolean }[];
  value: V;
  onChange: (value: V) => void;
  // Header controls at the end of the label row, e.g. the panel's Fast toggle
  actions?: ReactNode;
}

// An ordinal single-select (reasoning effort) as one segmented capsule with every level spelled out; the current segment is raised.
// The header carries only the label and `actions` at its end: the raised segment already names the current level, so it is not repeated.
// Segments size to their labels and share the spare width, so six Codex levels fit a --pop-w-lg panel on one line.
// Arrow keys move between segments (Base UI radio group)
export function SegmentScale<V extends string>({ label, options, value, onChange, actions }: SegmentScaleProps<V>) {
  return (
    <div className="flex flex-col gap-1 px-2 pb-1">
      <div className="flex min-h-ctl-sm items-center gap-2 text-2">
        <span className="min-w-0 flex-1 truncate text-fg-2">{label}</span>
        {actions && <span className="flex shrink-0 items-center gap-1">{actions}</span>}
      </div>
      <RadioGroup value={value} onValueChange={v => onChange(v as V)} aria-label={label}
        className="flex min-w-0 gap-0.5 rounded-md bg-hover p-0.5">
        {options.map(o => {
          const on = o.value === value;
          return (
            <Radio.Root render={<button type="button" />} nativeButton
              key={o.value}
              value={o.value}
              disabled={o.disabled}
              title={o.label}
              onKeyDownCapture={event => {
                // Enter activates a segment as Space does, matching RadioPills
                if (event.key === 'Enter') {
                  event.preventDefault();
                  event.currentTarget.click();
                }
              }}
              className={cn(
                'inline-flex h-ctl-sm min-w-0 flex-auto items-center justify-center gap-0.5 rounded-sm px-1 text-3 outline-none transition-colors focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-fg-2 disabled:opacity-50',
                o.ultra
                  ? on ? 'bg-ultra/15 text-ultra' : 'text-fg-3 enabled:hover:bg-hover enabled:hover:text-ultra'
                  : on ? 'bg-active text-fg-1' : 'text-fg-2 enabled:hover:bg-hover enabled:hover:text-fg-1',
              )}
            >
              <span className="truncate">{o.label}</span>
            </Radio.Root>
          );
        })}
      </RadioGroup>
    </div>
  );
}

export interface HeaderToggleProps {
  label: string;
  icon?: ReactNode;
  pressed: boolean;
  disabled?: boolean;
  onChange: (pressed: boolean) => void;
}

// A small pressed/unpressed button for a field header (the Fast toggle beside the effort level): --ctl-sm tall, --r-sm, like a toolbar Chip.
// Off it is an outlined ghost; on it is raised, and its icon takes the colour given by the caller
export function HeaderToggle({ label, icon, pressed, disabled, onChange }: HeaderToggleProps) {
  return (
    <button type="button" aria-pressed={pressed} disabled={disabled} onClick={() => onChange(!pressed)}
      className={cn(
        'inline-flex h-ctl-sm shrink-0 items-center gap-1 rounded-sm px-1.5 text-3 outline-none ring-1 ring-inset transition-colors focus-visible:ring-fg-2 disabled:opacity-50',
        pressed ? 'bg-active text-fg-1 ring-line-strong' : 'text-fg-3 ring-line enabled:hover:bg-hover enabled:hover:text-fg-1',
      )}>
      {icon}
      {label}
    </button>
  );
}
