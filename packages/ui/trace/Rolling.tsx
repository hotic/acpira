import type { CSSProperties } from 'react';
import { cn } from '../ui/cn';

const DIGITS = ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'];

// A figure whose digits roll to their new value (trace.css .trace-roll): each digit is a column of 0–9 slid to its place,
// keyed from the right so the ones stay the ones when the figure grows a digit. Other characters change in place
export function Rolling({ text, className }: { text: string; className?: string }) {
  const chars = [...text];
  return (
    <span className={cn('trace-roll', className)} aria-label={text} role="text">
      {chars.map((c, i) => {
        const place = chars.length - i;
        if (c < '0' || c > '9') return <span key={`${place}:${c}`} aria-hidden>{c}</span>;
        return (
          <span key={place} className="trace-digit" aria-hidden>
            <span className="trace-digit-strip" style={{ '--digit': c } as CSSProperties}>
              {DIGITS.map(d => <span key={d}>{d}</span>)}
            </span>
          </span>
        );
      })}
    </span>
  );
}
