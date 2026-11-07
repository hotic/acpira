// What the context report points at in the transcript: the calls under the hovered or picked node (their toolCallIds,
// ContextNode.call). The report writes it, the thread lights those rows and dims the rest (chat/reportLink.ts). Module
// state rather than context: the panel and the thread sit in different branches, and neither should re-render for it
export interface ReportLink {
  session: string;
  calls: string[];
  // Set while the calls are the picked node's: a new number brings the row into view once
  reveal?: number;
}

let current: ReportLink | null = null;
const listeners = new Set<() => void>();

export function setReportLink(next: ReportLink | null): void {
  const same = next === current || (!!next && !!current && next.session === current.session && next.reveal === current.reveal
    && next.calls.length === current.calls.length && next.calls.every((c, i) => c === current!.calls[i]));
  if (same) return;
  current = next;
  for (const l of listeners) l();
}

export function getReportLink(): ReportLink | null {
  return current;
}

export function subscribeReportLink(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
