import type { QueuedPrompt, Turn } from '@acpira/shared/transcript';

// Only the shown session supplies history; automatic instructions and attachment-only prompts are not typed messages.
export function promptHistory(turns: readonly Turn[], queued: readonly QueuedPrompt[] = []): string[] {
  const entries: string[] = [];
  const add = (text: string) => { if (text.trim()) entries.push(text); };
  for (const turn of turns) {
    if (turn.role === 'user') {
      if (!turn.auto && !turn.planId) add(turn.text);
    } else {
      for (const block of turn.blocks) if (block.type === 'steer') add(block.text);
    }
  }
  for (const prompt of queued) add(prompt.text);
  return entries;
}

// One instance per composer field (remounted on session switches). Snapshot on entry so live queue / transcript pushes
// cannot shift the cursor. Typing ends recall; the next empty-field Up starts again from the latest session history.
export class PromptRecall {
  private entries: readonly string[] | undefined;
  private index = 0;

  reset(): void { this.entries = undefined; }

  move(history: readonly string[], text: string, direction: -1 | 1): string | undefined {
    if (this.entries && text !== this.entries[this.index]) this.reset();
    if (!this.entries) {
      if (direction !== -1 || text !== '' || !history.length) return undefined;
      this.entries = [...history];
      this.index = this.entries.length;
    }
    this.index = Math.max(0, this.index + direction);
    if (this.index >= this.entries.length) { this.reset(); return ''; }
    return this.entries[this.index];
  }
}
