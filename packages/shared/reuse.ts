import type { SessionView } from './transcript';

// postMessage hands the webview a freshly cloned SessionView on every push, so no
// subtree keeps its identity even when nothing in it changed. Walk the new value
// against the previous one and return the previous reference wherever the two are
// deep-equal; memoized turn / block components then skip unchanged history.
export function reuse<T>(previous: T, next: T): T {
  if (previous === next) return next;
  if (Array.isArray(previous) && Array.isArray(next)) {
    let same = previous.length === next.length;
    const out = next.map((item, index) => {
      const kept = index < previous.length ? reuse(previous[index], item) : item;
      if (kept !== previous[index]) same = false;
      return kept;
    });
    return (same ? previous : out) as T;
  }
  if (!isPlainObject(previous) || !isPlainObject(next)) return next;
  const previousKeys = Object.keys(previous);
  const nextKeys = Object.keys(next);
  let same = previousKeys.length === nextKeys.length;
  const out: Record<string, unknown> = {};
  for (const key of nextKeys) {
    const kept = key in previous ? reuse(previous[key], next[key]) : next[key];
    if (!(key in previous) || kept !== previous[key]) same = false;
    out[key] = kept;
  }
  return (same ? previous : out) as T;
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  if (typeof value !== 'object' || value === null) return false;
  const proto = Object.getPrototypeOf(value);
  return proto === Object.prototype || proto === null;
}

// Keep the newer snapshot when two session payloads race. Missing rev (LAB fixtures, first paint)
// always applies so an unversioned host still updates.
export function applySession(current: SessionView | undefined, next: SessionView): SessionView {
  if (current?.id === next.id && next.rev != null && current.rev != null && next.rev <= current.rev) return current;
  return current?.id === next.id ? reuse(current, next) : next;
}
