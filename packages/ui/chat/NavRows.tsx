import type { ReactNode, Ref } from 'react';
import { Search } from 'lucide-react';
import { cn } from '../ui/cn';

// The left column's action rows, shared by the session column and the settings rail so both read as one list. They copy
// the session rows' box (height, inset, radius, lead slot); the lead takes the session rows' half-gap pull too: without
// it "New session" and "Search" started their text 6 px right of the pinned and project rows below them
export const NAV_ROW = 'flex min-h-row w-full items-center gap-gap rounded-md px-2 text-left text-2 text-fg-1 transition-colors hover:bg-hover focus-visible:bg-hover data-[popup-open]:bg-hover';
export const NAV_LEAD = '-mr-gap-half flex size-lead shrink-0 items-center justify-center text-fg-2';

// A keyboard shortcut at the end of a nav row
export function NavHint({ children }: { children: ReactNode }) {
  return <span className="shrink-0 text-3 text-fg-3 tabular-nums">{children}</span>;
}

// The column's search: a row like the ones around it, not a boxed field. `quiet`: until focused its placeholder reads
// like the row above (desktop), its shortcut at the end
export function NavSearch({ value, onChange, placeholder, label, autoFocus, quiet, hint, inputRef, className, part }: {
  value: string;
  onChange: (v: string) => void;
  placeholder: string;
  label: string;
  autoFocus?: boolean;
  quiet?: boolean;
  hint?: ReactNode;
  inputRef?: Ref<HTMLInputElement>;
  className?: string;
  // A plugin style hook (`data-part`)
  part?: string;
}) {
  return (
    <label data-part={part} className={cn(NAV_ROW, 'cursor-text text-fg-3 focus-within:bg-hover', className)}>
      <span className={NAV_LEAD}><Search className="size-icon" strokeWidth={1.5} /></span>
      <input
        ref={inputRef}
        autoFocus={autoFocus}
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder={placeholder}
        aria-label={label}
        className={cn('min-w-0 flex-1 bg-transparent text-2 text-fg-1 outline-none', quiet ? 'placeholder:text-fg-1 focus:placeholder:text-fg-3' : 'placeholder:text-fg-3')}
      />
      {hint && <NavHint>{hint}</NavHint>}
    </label>
  );
}

// A list's empty state in the rows' geometry: an empty lead slot (with the rows' pull), then the words on the titles'
// text edge at the rows' size, so it reads as the list's one row rather than a footnote under it
export function NavEmpty({ children }: { children: ReactNode }) {
  return <div className="flex min-h-row items-center gap-gap px-2 text-2 text-fg-3"><span className="-mr-gap-half w-lead shrink-0" />{children}</div>;
}
