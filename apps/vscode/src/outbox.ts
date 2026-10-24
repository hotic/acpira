import type { HostMsg } from '@shared/protocol';
import { applySessionPatch, mergeSessionPatches } from '@shared/sessionPatch';

// Messages that only ever describe current state: a later one makes an earlier one of its kind redundant
const STATE: ReadonlySet<HostMsg['type']> = new Set(['session', 'sessionPatch', 'sessions']);

// Queue `m` behind what is already waiting. Session state is idempotent, so a newer whole view replaces the session pushes
// still queued, a patch folds into the push queued before it, and the session list keeps only its latest copy. Any other
// message is a barrier: nothing is merged across it, so it still sees exactly the state that preceded it.
// The page keeps the last view of recent sessions to be patched against when it switches back (webview/sessionViews.ts),
// so a whole view drops only what it makes moot: pushes of its own session, and whole views of sessions switched away
// from (with the patches built on them; the page then asks for that session whole when it comes back). Patches of other
// sessions still go: dropping them would leave the page's kept copy behind the host's record of it
export function enqueue(queue: HostMsg[], m: HostMsg): void {
  let start = queue.length;
  while (start > 0 && STATE.has(queue[start - 1]!.type)) start--;
  if (m.type === 'session') {
    const dropped = new Set([m.session.id]);
    for (let i = start; i < queue.length; i++) {
      const q = queue[i]!;
      if (q.type === 'session') dropped.add(q.session.id);
      else if (q.type !== 'sessionPatch' || !dropped.has(q.patch.id)) continue;
      queue.splice(i--, 1);
    }
  } else if (m.type === 'sessions') {
    for (let i = queue.length - 1; i >= start; i--) if (queue[i]!.type === 'sessions') queue.splice(i, 1);
  } else if (m.type === 'sessionPatch') {
    for (let i = queue.length - 1; i >= start; i--) {
      const q = queue[i]!;
      if (q.type === 'session') {
        const view = q.session.id === m.patch.id ? applySessionPatch(q.session, m.patch) : undefined;
        if (view) { queue[i] = { type: 'session', session: view }; return; }
        break;
      }
      if (q.type === 'sessionPatch') {
        const merged = mergeSessionPatches(q.patch, m.patch);
        if (merged) { queue[i] = { type: 'sessionPatch', patch: merged }; return; }
        break;
      }
    }
  }
  queue.push(m);
}

// A post slower than this is reported (rate-limited): the webview link is the bottleneck, not the engine
const SLOW_MS = 2000;
const REPORT_EVERY_MS = 30_000;

// Messages to one webview with at most one postMessage in flight. Over Remote-SSH every post crosses the network, and
// without this a stream of whole session views queued up behind itself: the page fell further and further behind, and a
// model switch showed ~20 s after the click. Waiting for each post lets `enqueue` collapse what piles up meanwhile
export class PostQueue {
  private queue: HostMsg[] = [];
  private busy = false;
  private reported = 0;
  // Bumped by clear(): a post still in flight for the page being replaced no longer holds up the new one
  private generation = 0;

  constructor(
    private post: (m: HostMsg) => PromiseLike<unknown>,
    private log: (line: string) => void = () => {},
    // A post the webview never acknowledges (a page torn down mid-post) must not stall the rest forever
    private timeoutMs = 10_000,
    // Each post's outcome: delivered false when the webview refused it (postMessage answered false or failed)
    private onResult: (m: HostMsg, delivered: boolean) => void = () => {},
  ) {}

  push(m: HostMsg) {
    enqueue(this.queue, m);
    this.pump();
  }

  // The page is about to reload: nothing queued for the old one matters
  clear() {
    this.queue = [];
    this.busy = false;
    this.generation++;
  }

  get pending(): number { return this.queue.length; }

  private pump() {
    if (this.busy) return;
    const m = this.queue.shift();
    if (!m) return;
    this.busy = true;
    const generation = this.generation;
    const started = Date.now();
    let done = false;
    const finish = (delivered: boolean) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      if (generation !== this.generation) return;
      const ms = Date.now() - started;
      if (ms >= SLOW_MS && Date.now() - this.reported >= REPORT_EVERY_MS) {
        this.reported = Date.now();
        this.log(`webview post of ${m.type} took ${ms} ms (${this.queue.length} queued behind it)`);
      }
      this.onResult(m, delivered);
      this.busy = false;
      this.pump();
    };
    // A timeout only stops waiting: the post itself is still on its way, in order
    const timer = setTimeout(() => finish(true), this.timeoutMs);
    // A clear() before this runs means the message was for the page being replaced: it is not sent at all
    Promise.resolve().then(() => generation === this.generation ? this.post(m) : true).then(ok => finish(ok !== false), () => finish(false));
  }
}
