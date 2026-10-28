import type { DiffLine, DiffSource } from '@acpira/shared/transcript';
import type { CodeDiffRow } from './codeDiff';
import { highlightDiff } from './codeDiffHighlight';
import { tokenizeCode, type CodeTheme, type CodeToken, type Language } from './codeSyntax';

// `theme`: the scheme the tokens get their colours for; `parts`: the lines tokenized so far are sent ahead of the whole
// result (`part`), for a file shown while it colours; `codes`: many short sources (a file's removed hunks) as one run,
// each from the grammar's first state; `cancel`: the request `id` is no longer wanted (its view went away)
export type HighlightRequest =
  | { id: number; kind: 'diff'; lines: DiffLine[]; source?: DiffSource; path: string; theme: CodeTheme }
  | { id: number; kind: 'code'; text: string; language: Language; theme: CodeTheme; parts?: boolean }
  | { id: number; kind: 'codes'; texts: string[]; language: Language; theme: CodeTheme }
  | { id: number; kind: 'cancel' };
export type HighlightResult = CodeDiffRow[] | CodeToken[][] | CodeToken[][][];
export type HighlightResponse = { id: number; result: HighlightResult; error?: never; part?: never }
  | { id: number; result?: never; error: string; part?: never }
  | { id: number; result?: never; error?: never; part: CodeToken[][] };

// The first part goes out once it holds a screen of lines (SCREEN_LINES) or the run has been going FIRST_PART_MAX_MS,
// never before FIRST_PART_MS (a source done by then sends its whole result only); the next parts every PART_MS
const SCREEN_LINES = 64;
const FIRST_PART_MS = 50;
const FIRST_PART_MAX_MS = 150;
const PART_MS = 200;

// A macrotask turn without setTimeout's clamping: the messages queued meanwhile run before it resolves
const turn = new MessageChannel();
const turns: (() => void)[] = [];
turn.port1.onmessage = () => turns.shift()?.();
const nextTurn = () => new Promise<void>(resolve => {
  turns.push(resolve);
  turn.port2.postMessage(0);
});

// Every request starts as it arrives and the runs take turns (`tokenizeCode` pauses after each slice), so a short one (a
// file just opened) ends within a few turns however long the others are. A run that has gone TURN_MS since its last turn
// lets the queued messages in at its next pause, and a cancelled one ends there
const TURN_MS = 16;
const runs = new Map<number, { cancelled: boolean }>();
class Cancelled extends Error {}

// Grammar initialization and full-source tokenization must not occupy the UI
// thread during disclosure animation or streaming. One worker shares loaded grammars
// across diffs and fenced code blocks.
self.onmessage = async ({ data }: MessageEvent<HighlightRequest>) => {
  if (data.kind === 'cancel') {
    const run = runs.get(data.id);
    if (run) run.cancelled = true;
    return;
  }
  const run = { cancelled: false };
  runs.set(data.id, run);
  let turnAt = performance.now();
  const pause = async () => {
    if (performance.now() - turnAt >= TURN_MS) {
      await nextTurn();
      turnAt = performance.now();
    }
    if (run.cancelled) throw new Cancelled();
  };
  let response: HighlightResponse;
  try {
    const started = performance.now();
    let sent = 0;
    let due = started + FIRST_PART_MS;
    const progress = data.kind === 'code' && data.parts ? (lines: readonly CodeToken[][]) => {
      const now = performance.now();
      if (now < due || (!sent && lines.length < SCREEN_LINES && now - started < FIRST_PART_MAX_MS)) return;
      self.postMessage({ id: data.id, part: lines.slice(sent) } satisfies HighlightResponse);
      sent = lines.length;
      due = now + PART_MS;
    } : undefined;
    let result: HighlightResult;
    if (data.kind === 'diff') result = await highlightDiff(data.lines, data.theme, data.source, data.path, pause);
    else if (data.kind === 'code') result = await tokenizeCode(data.text, data.language, data.theme, pause, progress);
    else {
      const each: CodeToken[][][] = [];
      for (const text of data.texts) each.push(await tokenizeCode(text, data.language, data.theme, pause));
      result = each;
    }
    response = { id: data.id, result };
  } catch (error) {
    if (error instanceof Cancelled) return;
    response = { id: data.id, error: error instanceof Error ? error.message : String(error) };
  } finally {
    runs.delete(data.id);
  }
  self.postMessage(response);
};
