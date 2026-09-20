import { randomUUID } from 'node:crypto';
import type { Attachment, Draft, QueuedPrompt } from '@shared/transcript';
import { preparePrompt, restoreDrafts, type BlobStore, type PreparedPrompt, type PromptCaps } from './attachments';
import { msg } from '../errors';
import { t } from '../i18n';

// A prompt waiting for the current turn to finish. Staged the moment it is queued (blobs written, image files read), so the queue
// row can show the attachments like a sent turn's and the flush has no second pass over the drafts; `problems` are reported at queue time
interface StagedPrompt {
  id: string;
  text: string;
  prepared: PreparedPrompt;
}

// How `prompt` receives an already staged payload: the queue flush hands its entry over, an edited turn also marks the user turn
export interface StagedSend {
  prepared: PreparedPrompt;
  edited?: boolean;
}

export interface PromptQueueDeps {
  sessionId: string;
  blobs: BlobStore;
  log: (line: string) => void;
  notify?: (text: string) => void;
  bump: () => void;
  touch: () => void;
  isReady: () => boolean;
  isRunning: () => boolean;
  // starting sessions accept a first prompt into the queue; flush waits for ready
  canEnqueue: () => boolean;
  // The agent's prompt capabilities at staging time, read off the live proc so a queued entry sees them as late as possible
  caps: () => PromptCaps | undefined;
  send: (text: string, prepared: PreparedPrompt) => Promise<void>;
}

export class PromptQueue {
  private items: StagedPrompt[] = [];
  private sendingId?: string;

  constructor(private deps: PromptQueueDeps) {}

  snapshot(): QueuedPrompt[] | undefined {
    if (!this.items.length) return undefined;
    return this.items.map(q => ({ id: q.id, text: q.text, attachments: q.prepared.attachments,
      ...(q.id === this.sendingId ? { sending: true } : {}),
    }));
  }

  clear() { this.items = []; this.sendingId = undefined; }

  // Reserve the selected entry before cancelling the active turn. Repeated clicks cannot cancel its replacement turn.
  prioritize(id: string): boolean {
    if (this.sendingId || !this.deps.isReady()) return false;
    const index = this.items.findIndex(q => q.id === id);
    if (index < 0) return false;
    const [entry] = this.items.splice(index, 1);
    this.items.unshift(entry!);
    this.sendingId = id;
    this.deps.touch();
    return true;
  }

  release(id: string) {
    if (this.sendingId !== id) return;
    this.sendingId = undefined;
    this.deps.touch();
  }

  // Send the first prompt queued during the last turn, if any; nobody awaits it, so its failures end up in the log. The rest stay queued behind it
  flush(): boolean {
    if (!this.deps.isReady() || this.deps.isRunning()) return false;
    const next = this.items.shift();
    if (!next) return false;
    this.sendingId = undefined;
    this.deps.send(next.text, next.prepared).catch(e => this.deps.log(`queued prompt failed: ${msg(e)}`));
    return true;
  }

  // Queue a prompt behind the running turn (or while the session is still starting). Staging happens now so the row above the composer can
  // show attachments; a staging failure keeps the text alone. The turn may end while staging: then the entry goes straight out, since the
  // turn's own flush found the queue empty
  async enqueue(text: string, attachments: Draft[], staged?: PreparedPrompt): Promise<void> {
    const prepared = staged ?? await this.stage(text, attachments);
    if (!prepared.blocks.length || !this.deps.canEnqueue()) return;
    this.items.push({ id: randomUUID(), text, prepared });
    this.deps.bump();
    if (!this.deps.isRunning()) this.flush();
  }

  // Drop a queued prompt; a no-op when it already went out
  dequeue(id: string) {
    if (id === this.sendingId) return;
    const before = this.items.length;
    this.items = this.items.filter(q => q.id !== id);
    if (this.items.length !== before) this.deps.touch();
  }

  // Replace a queued prompt in place: kept attachments come back from their blobs, new drafts are staged alongside. Emptying it removes it
  async editQueued(id: string, text: string, retained: number[], drafts: Draft[]): Promise<void> {
    if (id === this.sendingId) return;
    const entry = this.items.find(q => q.id === id);
    if (!entry) throw new Error(t('queue.gone'));
    const kept = retained.map(i => entry.prepared.attachments[i]).filter((a): a is Attachment => !!a);
    const prepared = await this.stage(text, [...await restoreDrafts(this.deps.sessionId, kept, this.deps.blobs), ...drafts]);
    // It may have gone out while the blobs were being read
    if (!this.items.includes(entry) || id === this.sendingId) throw new Error(t('queue.gone'));
    if (!prepared.blocks.some(b => b.type !== 'text' || b.text.trim())) { this.dequeue(id); return; }
    entry.text = text;
    entry.prepared = prepared;
    this.deps.touch();
  }

  private async stage(text: string, attachments: Draft[]): Promise<PreparedPrompt> {
    let prepared: PreparedPrompt;
    try { prepared = await preparePrompt(this.deps.sessionId, text, attachments, this.deps.blobs, this.deps.caps()); }
    catch (e) {
      this.deps.log(`Attachment staging failed: ${msg(e)}`);
      this.deps.notify?.(t('host.attachFailed', { error: msg(e) }));
      return { blocks: text.trim() ? [{ type: 'text', text }] : [], attachments: [], problems: [] };
    }
    for (const p of prepared.problems) { this.deps.log(p); this.deps.notify?.(p); }
    return { ...prepared, problems: [] };
  }
}
