import { useRef, useState } from 'react';
import { Check, X } from 'lucide-react';
import type { ToolStatus } from '@acpira/shared/transcript';
import { useEntranceEnd } from '../ui/Row';
import { commandDuration } from './commandDuration';

// A command's outcome at its row's end. The ✓ / ✗ slot is held while the command runs, so the glyph's arrival moves
// nothing on the row; a command that ends on screen draws its glyph and fades its final run time in over the ticking one.
// One already ended when the row mounted stays still
export function CommandOutcome({ status, duration, glyph = true }: { status: ToolStatus; duration?: string; glyph?: boolean }) {
  const done = status === 'completed' || status === 'failed';
  const [ended, setEnded] = useState(false);
  const wasRunning = useRef(!done);
  if (done && wasRunning.current) {
    wasRunning.current = false;
    if (!ended) setEnded(true);
  }
  const mark = useRef<HTMLSpanElement>(null);
  const time = useRef<HTMLSpanElement>(null);
  // Dropped once the last part lands (the glyph's draw, else the time's fade)
  useEntranceEnd(ended, glyph ? mark : time, glyph ? ['cmd-pop', 'cmd-draw'] : ['cmd-time-in'], () => setEnded(false));
  const enter = ended ? '' : undefined;
  return <>
    {glyph && <span ref={mark} className="cmd-outcome inline-flex size-3 shrink-0" data-enter={enter}>
      {status === 'completed' && <Check className="size-3 text-ok" strokeWidth={2} aria-hidden="true" />}
      {status === 'failed' && <X className="size-3 text-danger" strokeWidth={2} aria-hidden="true" />}
    </span>}
    {/* The time's slot keeps at least the width of the widest time below a minute ("<0.1 s"), so the ticking whole
        seconds turning into the final tenths never shift the command text beside them */}
    {duration && <span className="inline-grid justify-items-end">
      <span aria-hidden className="invisible col-start-1 row-start-1">{commandDuration(50)}</span>
      <span ref={time} key={done ? 'final' : 'live'} className="cmd-time col-start-1 row-start-1" data-enter={done ? enter : undefined}>{duration}</span>
    </span>}
  </>;
}
