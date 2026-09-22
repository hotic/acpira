import type { HostMsg } from '@shared/protocol';

// Manager events within this window collapse to one post per message type
export const BATCH_WINDOW_MS = 30;

// Streaming session views share a live turns array. Clone the snapshot at post time so an in-flight
// structured clone cannot pick up endTurn mutations (running:true with a sealed last turn).
export function freezeHostMsg(msg: HostMsg): HostMsg {
  if (msg.type === 'session') return { type: 'session', session: structuredClone(msg.session) };
  if (msg.type === 'subagent') return { ...msg, turns: structuredClone(msg.turns) };
  if (msg.type === 'init' && msg.state.active) {
    return { ...msg, state: { ...msg.state, active: structuredClone(msg.state.active) } };
  }
  return msg;
}

// Several subagent streams can be live at once per session; keying on type alone would collapse them into one
function batchKey(m: HostMsg): string {
  return m.type === 'subagent' ? `subagent:${m.sessionId}:${m.subagentId}` : m.type;
}

export class HostMsgBatch {
  private pending = new Map<string, HostMsg>();
  private timer?: ReturnType<typeof setTimeout>;

  constructor(
    private readonly post: (m: HostMsg) => void,
    private readonly windowMs = BATCH_WINDOW_MS,
  ) {}

  push(m: HostMsg) {
    this.pending.set(batchKey(m), m);
    // An idle edge must not wait out the window: a later running:true clone already in flight is
    // the stuck-spinner case, and delaying idle makes it more likely to lose the race.
    if (m.type === 'session' && !m.session.running) this.flush();
    else this.timer ??= setTimeout(() => this.flush(), this.windowMs);
  }

  flush() {
    if (this.timer !== undefined) {
      clearTimeout(this.timer);
      this.timer = undefined;
    }
    if (!this.pending.size) return;
    const queued = [...this.pending.values()];
    this.pending.clear();
    for (const msg of queued) this.post(msg);
  }

  dispose() {
    if (this.timer !== undefined) clearTimeout(this.timer);
    this.pending.clear();
  }
}
