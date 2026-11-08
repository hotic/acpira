import { Attachment, FileHit } from '../wire/Wire';
import { TurnEdit } from '../logic/Composing';

// What the composer keeps in the app model (apps/ios Store/AppModel+Composer.swift, apps/android store/Composing.kt):
// each session's unsent draft, the `@` file search the engine answers, and the liquid picker that is open over the
// composer (Design/Goo.swift), which the chat page draws over its whole screen.

/** One chat's unsent prompt and the attachments staged with it; an object per session, so typing redraws the composer */
@ObservedV2
export class ComposerDraft {
  @Trace text: string = '';
  @Trace staged: Attachment[] = [];
  /** A sent prompt the composer is rewriting */
  @Trace edit: TurnEdit | undefined = undefined;
  /** Bumped to ask the field for focus (a prompt taken in for editing, a queued prompt taken back) */
  @Trace focusKey: number = 0;
  /** An edit sent and not yet answered: its request id, and what goes back into the composer if the engine refuses it */
  pending: PendingEdit | undefined = undefined;

  get isEmpty(): boolean {
    return this.text.length === 0 && this.staged.length === 0 && this.edit === undefined && this.pending === undefined;
  }
}

export interface PendingEdit {
  request: string;
  edit: TurnEdit;
  text: string;
  staged: Attachment[];
}

/**
 * The `@` list's file search: each query goes out with a new seq and only the reply to the latest one lands; closing
 * the list invalidates whatever is in flight. Without an engine the demo project's files answer at once
 */
@ObservedV2
export class FileSearch {
  @Trace hits: FileHit[] = [];
  /** The query the hits answer (undefined until the first reply, so an empty query is not taken as answered) */
  @Trace answered: string | undefined = undefined;
  seq: number = 0;
  private asked: string | undefined = undefined;
  /** Posts `searchFiles`; false without an engine */
  private post: (query: string, seq: number) => boolean;

  constructor(post: (query: string, seq: number) => boolean) {
    this.post = post;
  }

  search(query: string): void {
    this.seq++;
    this.asked = query;
    if (!this.post(query, this.seq)) {
      this.receive(this.seq, demoFiles(query));
    }
  }

  /** The engine's `files` reply; one for an older seq is dropped */
  receive(seq: number, files: FileHit[]): void {
    if (seq !== this.seq || this.asked === undefined) {
      return;
    }
    this.hits = files;
    this.answered = this.asked;
  }

  close(): void {
    this.seq++;
    this.asked = undefined;
    if (this.answered !== undefined || this.hits.length > 0) {
      this.hits = [];
      this.answered = undefined;
    }
  }
}

const DEMO_ROOT = '/Users/dev/projects/acpira';
const DEMO_PATHS: string[] = [
  'README.md', 'package.json', 'tsconfig.json', 'src/args.ts', 'src/foo.ts', 'src/cli/args.ts', 'src/cli/index.ts',
  'docs/dev/ios.md', 'docs/dev/webview.md', 'packages/ui/chat/Composer.tsx', 'packages/shared/protocol.ts',
  'apps/harmony/entry/src/main/ets/components/composer/Composer.ets', 'rust/crates/acpira-host/src/runtime.rs',
  'assets/screenshot.png'
];

/** The demo's stand-in for the engine's walk of the project folder: names holding the query first, then paths */
function demoFiles(query: string): FileHit[] {
  const q = query.toLowerCase();
  const base = (p: string): string => p.split('/').pop() ?? p;
  const named = DEMO_PATHS.filter((p: string) => q.length === 0 || base(p).toLowerCase().includes(q));
  const rest = DEMO_PATHS.filter((p: string) => !named.includes(p) && p.toLowerCase().includes(q));
  return named.concat(rest).slice(0, 20).map((p: string) => {
    const hit: FileHit = { uri: `file://${DEMO_ROOT}/${p}`, path: p };
    return hit;
  });
}

/** Where an attach bean sends the reader */
export enum AttachSource {
  Photos,
  Camera,
  Files,
  Paste
}

/**
 * The open liquid picker (Goo.swift GooChip / LiquidControls state): which chip's beans are out (`attach` or `model`),
 * whose panel is open (1…, 0 for none), and the sidekick's level of beans in place of the lead's. The composer's chips
 * report where they sit, so the screen's layer raises the beans out of them
 */
@ObservedV2
export class Goo {
  @Trace open: string | undefined = undefined;
  @Trace stage: number = 0;
  @Trace deeper: string | undefined = undefined;
  /** Where the chips sit in window vp (their left edges and their shared top), so the beans rise out of them */
  @Trace attachLeft: number = 0;
  @Trace modelLeft: number = 0;
  @Trace chipTop: number = 0;
  /** The chip opened last, which a closing picker goes home into */
  @Trace last: string = 'model';
  /** The source a bean asked for, picked up by the composer's pickers */
  @Trace source: AttachSource | undefined = undefined;

  /** The value of `deeper` while the sidekick's beans are out */
  static readonly sidekickLevel: string = 'sidekick';

  toggle(id: string): void {
    if (this.open === id) {
      this.back();
    } else {
      this.open = id;
      this.last = id;
      this.stage = 0;
      this.deeper = undefined;
    }
  }

  /** One level back: an open panel into its bean, the sidekick's beans for the lead's, else the beans into the chip */
  back(): void {
    if (this.stage > 0) {
      this.stage = 0;
    } else if (this.deeper !== undefined) {
      this.deeper = undefined;
    } else {
      this.close();
    }
  }

  close(): void {
    this.open = undefined;
    this.stage = 0;
  }

  /** A bean's panel in or out */
  tap(index: number): void {
    this.stage = this.stage === index + 1 ? 0 : index + 1;
  }
}
