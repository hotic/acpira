import type * as acp from '@agentclientprotocol/sdk';

export function isCompactCommand(text: string): boolean {
  return /^\/compact(?:\s|$)/.test(text.trim());
}

// A prompt acknowledgement is not necessarily the end of compaction. Devin and
// Kimi run /compact in the background and report its result as plain text. Arm
// their latch before sending: Devin may acknowledge before its first text chunk.
// Other peers use the RPC lifetime plus any structured compaction_update events.
export class CompactionCompletion {
  private pending = new Set<string>();
  private manual: boolean;
  private structured = false;
  private text = '';
  private release?: () => void;
  // Post-compaction token count self-reported in the completion prose ("- Tokens after: N");
  // agents that run /compact in the background push no usage_update until the next turn
  tokensAfter?: number;

  constructor(private agent?: string) {
    this.manual = agent === 'devin' || agent === 'kimi';
  }

  update(u: acp.SessionUpdate) {
    if (u.sessionUpdate === 'compaction_update') {
      this.structured = true;
      this.manual = false;
      if (['completed', 'failed', 'cancelled'].includes(u.status)) this.pending.delete(u.compactionId);
      else this.pending.add(u.compactionId);
    } else if (this.agent && !this.structured && u.sessionUpdate === 'agent_message_chunk' && u.content.type === 'text') {
      // Only parse an explicitly issued /compact for the two known adapters;
      // ordinary model prose must never acquire or release a compaction latch.
      this.text = (this.text + u.content.text).slice(-4096);
      // The token line can land after the terminal marker already released the latch, so parse on every
      // chunk and keep overwriting: mid-chunk the capture is a prefix of the number and converges as the line completes
      const after = /- Tokens after:\s*([\d,]+)/.exec(this.text)?.[1];
      if (after) this.tokensAfter = Number(after.replaceAll(',', ''));
      if (this.manual) {
        const terminal = this.agent === 'devin'
          ? /Context compacted|Nothing to compact\.|(?:Force compaction|Compaction) failed:|Compaction cancel(?:ed|led)\./
          : /Compaction completed\.|Compaction cancelled\.|Compaction is blocked by the current turn;|\/compact failed:/;
        if (terminal.test(this.text)) this.manual = false;
      }
    }
    if (!this.manual && this.pending.size === 0) this.release?.();
  }

  wait(): Promise<void> | undefined {
    if (!this.manual && this.pending.size === 0) return;
    return new Promise(resolve => { this.release = resolve; });
  }

  // RPC failure, process exit, or session disposal ends the local wait as well.
  close() {
    this.manual = false;
    this.pending.clear();
    this.release?.();
  }
}
