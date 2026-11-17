import { useEffect, useState } from 'react';
import { durationText } from './folding';

// Running time of a live turn, next to its Working label, the bare duration like the finished head (35 分钟 12 秒). A leaf
// with its own one-second tick, so only this span re-renders; the memoized turn above it keeps the stream's render
// budget. Tabular digits keep the width steady between ticks (it only grows when a unit is added). Nothing shows during
// the first second (a bare "0s" next to Working says nothing); ticks land on whole seconds after `since`, so "1s"
// appears exactly when one second has passed
export function LiveElapsed({ since }: { since: number }) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    const tick = () => {
      const at = Date.now();
      setNow(at);
      // Delay to the next whole second since `since` (a clock behind `since` waits for the first one)
      const delay = 1000 - (Math.max(0, at - since) % 1000);
      timer = setTimeout(tick, delay);
    };
    tick();
    return () => clearTimeout(timer);
  }, [since]);
  if (now - since < 1000) return null;
  return <span className="min-w-0 shrink-0 truncate font-normal text-fg-3 tabular-nums">{durationText(now - since)}</span>;
}
