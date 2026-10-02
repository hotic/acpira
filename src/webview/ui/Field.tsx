import { useState, type ReactNode } from 'react';
import { ChevronDown } from 'lucide-react';
import { Switch } from './Switch';
import { Radio } from '@base-ui/react/radio';
import { RadioGroup } from '@base-ui/react/radio-group';
import { DropdownMenu } from './DropdownMenu';
import { OptionContent } from './Panel';
import { cn } from './cn';

// Neutral form controls for menu footers: switch rows, segmented single-select groups, ordinal step scales and inline dropdown rows.

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

export interface StepScaleProps<V extends string> {
  label: string;
  options: { value: V; label: string; disabled?: boolean }[];
  value: V;
  onChange: (value: V) => void;
}

// An ordinal single-select (reasoning effort) as one row of stops on a track, whatever the level count: six levels used to wrap
// a pill group onto a second line inside a 260 px panel. The header names the current level; pointing at a stop previews its name
// there (dimmed) before the click. The track is filled up to the current stop. Each cell draws its own half segments, so the line
// starts and ends at the outer stops' centres without any computed geometry. Arrow keys move between stops (Base UI radio group)
export function StepScale<V extends string>({ label, options, value, onChange }: StepScaleProps<V>) {
  const [hover, setHover] = useState<V>();
  const at = options.findIndex(o => o.value === value);
  const preview = hover !== undefined && hover !== value ? options.find(o => o.value === hover) : undefined;
  const shown = preview ?? options[at];
  const last = options.length - 1;
  return (
    <div className="flex flex-col px-2 pb-1">
      <div className="flex min-h-ctl-sm items-center gap-2 text-2">
        <span className="min-w-0 flex-1 truncate text-fg-2">{label}</span>
        <span className={cn('shrink-0 transition-colors', preview ? 'text-fg-3' : 'text-fg-1')} aria-hidden>{shown?.label}</span>
      </div>
      <RadioGroup value={value} onValueChange={v => onChange(v as V)} aria-label={label} className="flex min-w-0" onPointerLeave={() => setHover(undefined)}>
        {options.map((o, i) => (
          <Radio.Root render={<button type="button" />} nativeButton
            key={o.value}
            value={o.value}
            disabled={o.disabled}
            aria-label={o.label}
            title={o.label}
            onPointerEnter={() => setHover(o.value)}
            onKeyDownCapture={event => {
              // Enter activates a stop as Space does, matching RadioPills
              if (event.key === 'Enter') {
                event.preventDefault();
                event.currentTarget.click();
              }
            }}
            className="group relative flex h-ctl-sm min-w-0 flex-1 items-center justify-center rounded-sm outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-fg-2 disabled:opacity-50"
          >
            {/* Track halves: left of the stop is filled when the stop is at or below the current one, right when strictly below */}
            {i > 0 && <span aria-hidden className={cn('absolute left-0 right-1/2 h-0.5', i <= at ? 'bg-fg-2' : 'bg-active')} />}
            {i < last && <span aria-hidden className={cn('absolute left-1/2 right-0 h-0.5', i < at ? 'bg-fg-2' : 'bg-active')} />}
            <span aria-hidden className={cn(
              'relative rounded-full transition-[background-color,transform]',
              i === at ? 'size-3 bg-fg-1' : 'size-2 group-enabled:group-hover:scale-150',
              i !== at && (i < at ? 'bg-fg-2' : 'bg-fg-3 group-enabled:group-hover:bg-fg-2'),
            )} />
          </Radio.Root>
        ))}
      </RadioGroup>
    </div>
  );
}
