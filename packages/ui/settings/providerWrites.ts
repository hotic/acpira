import type { Provider, ProviderAction } from '@acpira/shared/providers';

type Write = { action: ProviderAction; optimistic: boolean };

// Provider replies carry whole snapshots without request ids. Keep writes ordered in the page's lifetime, not
// the settings dialog's, so closing the dialog cannot abandon a queued toggle or mistake an older reply for it.
export class ProviderWrites {
  private active?: Write;
  private queue: Write[] = [];
  private reading = false;

  constructor(private send: (action: ProviderAction) => void, private read: () => void) {}

  get pending(): boolean { return !!this.active || this.queue.length > 0; }

  refresh(): void {
    if (this.reading || this.pending) return;
    this.reading = true;
    this.read();
  }

  // Only row edits echo optimistically. A form still waits for its own successful reply before closing.
  get drafts(): Record<string, Provider> {
    const drafts: Record<string, Provider> = {};
    for (const write of [...(this.active ? [this.active] : []), ...this.queue]) {
      if (write.optimistic && write.action.kind === 'save') drafts[write.action.provider.id] = write.action.provider;
    }
    return drafts;
  }

  push(action: ProviderAction, optimistic = false): void {
    const write = { action, optimistic };
    if (!this.active && !this.reading) {
      this.active = write;
      this.send(action);
      return;
    }
    // Consecutive row edits of one source need only its newest snapshot. Never merge credential/form writes.
    const last = this.queue.at(-1);
    if (optimistic && last?.optimistic && action.kind === 'save' && last.action.kind === 'save' && action.provider.id === last.action.provider.id) {
      this.queue[this.queue.length - 1] = write;
    } else this.queue.push(write);
  }

  receive(): ProviderAction | undefined {
    // A read already in flight when the first toggle was clicked must not acknowledge that queued write.
    const settled = this.reading ? undefined : this.active?.action;
    this.reading = false;
    this.active = this.queue.shift();
    if (this.active) this.send(this.active.action);
    return settled;
  }
}
