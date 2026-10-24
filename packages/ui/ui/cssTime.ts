// Reading CSS time values in milliseconds.
// The production stylesheet is minified, and the minifier rewrites `320ms` as `.32s`: a bare `parseFloat` read the
// token as 0.32 ms there while the unminified LAB / dev build still said 320, so timers keyed to a transition fired
// at once in the shipped extension only (a closing fold released its body ~80 ms into the close and snapped shut).

// A CSS <time> (`320ms`, `.32s`, `0.32s`) in milliseconds; `undefined` for anything else (empty, `auto`, a calc())
export function parseCssTime(value: string): number | undefined {
  const match = /^\s*(-?(?:\d+\.?\d*|\.\d+))(ms|s)\s*$/i.exec(value);
  if (!match) return undefined;
  const amount = Number.parseFloat(match[1]!);
  return match[2]!.toLowerCase() === 's' ? amount * 1000 : amount;
}

// A time-valued custom property on `el` in milliseconds, or `fallback` when it is unset or unreadable
export function cssTimeVar(el: Element, name: string, fallback: number): number {
  return parseCssTime(getComputedStyle(el).getPropertyValue(name)) ?? fallback;
}
