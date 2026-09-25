import type { AgentTurn, CompactionBlock, CompactionStatus } from '@shared/transcript';

// Presentation only: ACP completion and queue release remain owned by the host.
// Apply this exclusively to replies to /compact. The host already turns adapter prose into compaction blocks
// (rust `compaction_text`); this keeps records persisted before that readable and covers the wait for the first chunk.
export function compactionForDisplay(turn: AgentTurn, running: boolean): AgentTurn {
  const text = turn.blocks.filter(b => b.type === 'text').map(b => b.markdown).join('').trim();
  const structured = turn.blocks.filter((b): b is CompactionBlock => b.type === 'compaction');
  const prefixes = ['Compacting context', 'Context compaction started', 'Compaction started', 'Context compacted', 'Compaction completed.', 'Nothing to compact.'];
  const known = /^(?:Compacting context(?:\.{3}|…|$)|Context compaction started\b|Compaction started\b|Context compacted\b|Compaction completed\.|Nothing to compact\.|(?:Force compaction|Compaction) failed:|\/compact failed:|Compaction cancel(?:ed|led)\.|Compaction is blocked by the current turn;)/.test(text)
    || (running && text.length > 0 && prefixes.some(prefix => prefix.startsWith(text)));
  if (!known && !structured.length && !(running && !turn.blocks.length)) return turn;

  let status: CompactionStatus = structured.at(-1)?.status ?? 'in_progress';
  if (!structured.length) {
    if (/(?:Force compaction|Compaction) failed:|\/compact failed:|Compaction is blocked by the current turn;/.test(text)) status = 'failed';
    else if (/Compaction cancel(?:ed|led)\./.test(text)) status = 'cancelled';
    else if (/Context compacted|Compaction completed\.|Nothing to compact\./.test(text)) status = 'completed';
    // An incomplete persisted response is not evidence of success or failure.
    else if (!running && turn.stop !== 'error' && turn.stop !== 'cancelled') return turn;
  }
  if (turn.stop === 'error') status = 'failed';
  if (turn.stop === 'cancelled') status = 'cancelled';
  const error = status === 'failed' ? structured.at(-1)?.error : undefined;
  const block: CompactionBlock = { type: 'compaction', id: structured[0]?.id ?? 'compact-display', status, ...(error ? { error } : {}) };
  // Known CLI responses include progress text and statistics; show a single status.
  // Preserve unrelated content and concrete failure details for diagnosis.
  const blocks = turn.blocks.filter(b => b.type !== 'compaction' && !(b.type === 'text' && known));
  if (known && status === 'failed' && text) blocks.push({ type: 'text', markdown: text });
  return { ...turn, blocks: [block, ...blocks] };
}
