import { SessionView, SlashCommand, Turn } from '../wire/Wire';
import { asView, SubagentSummary } from '../wire/Blocks';
import { isContextLengthError, shortStop } from '../logic/Composing';

// What the chat's rows read from the view beyond their own turn (apps/ios Chat/ThreadItems.swift passes it per item).
// A turn row keeps its ForEach key while its turn object stays the same, and a kept child keeps the params it was built
// with (docs/dev/harmony.md), so these reach the rows through this observed object rather than through params: a patch
// that changes only the view's subagents, controls or running state still reaches the rows of kept turns.

/** Turn objects a patch kept keep their id, so their rows keep their keys; a new or changed turn gets a new one */
const turnIds: WeakMap<Turn, number> = new WeakMap<Turn, number>();
let lastTurnId: number = 0;

/** The thread's ForEach key for `turn` at `i`: stable while the patch keeps the same object at the same place */
export function turnKey(turn: Turn, i: number): string {
  let id = turnIds.get(turn);
  if (id === undefined) {
    id = ++lastTurnId;
    turnIds.set(turn, id);
  }
  return `${i}:${id}`;
}

const NO_SUBAGENTS: SubagentSummary[] = [];
const NO_COMMANDS: string[] = [];

/** The view's controls as the plan card reads them */
interface ViewControls {
  controls?: Object;
}

@ObservedV2
export class ThreadContext {
  /** The view's subagents: every node, for counts and provenance */
  @Trace subagents: SubagentSummary[] = NO_SUBAGENTS;
  /** The session's controls (the plan card's execution model), read loosely */
  @Trace controls: Object | undefined = undefined;
  @Trace agent: string = '';
  /** The agent's advertised command names: the prompt bubbles tint them */
  @Trace commands: string[] = NO_COMMANDS;
  /** The running turn's index, -1 while idle */
  @Trace live: number = -1;
  /** The failed turn Retry answers (TurnEdit.swift retryableTurn), -1 for none */
  @Trace retry: number = -1;
  // Every patch parses its own copy of the view's fields: a field is written only when its content changed, so rows
  // that read it render again only then
  private subagentsSig: string = '';
  private controlsSig: string = '';
  private commandsSig: string = '';

  follow(view: SessionView | undefined): void {
    const subagents = view === undefined ? NO_SUBAGENTS : (asView(view).subagents ?? NO_SUBAGENTS);
    const subagentsSig = JSON.stringify(subagents);
    if (subagentsSig !== this.subagentsSig) {
      this.subagentsSig = subagentsSig;
      this.subagents = subagents;
    }
    const controls = view === undefined ? undefined : (view as Object as ViewControls).controls;
    const controlsSig = JSON.stringify(controls ?? null);
    if (controlsSig !== this.controlsSig) {
      this.controlsSig = controlsSig;
      this.controls = controls;
    }
    const commands = view === undefined ? NO_COMMANDS : (view.commands ?? []).map((c: SlashCommand) => c.name);
    const commandsSig = commands.join('\n');
    if (commandsSig !== this.commandsSig) {
      this.commandsSig = commandsSig;
      this.commands = commands;
    }
    this.agent = view?.agent ?? '';
    this.live = view !== undefined && view.running ? view.turns.length - 1 : -1;
    this.retry = view !== undefined ? retryableTurn(view) : -1;
  }
}

/** The failed turn Retry answers: an error that is not a context overflow, and that declares retry if it names remedies */
function retryableTurn(view: SessionView): number {
  const turn = shortStop(view);
  if (turn === undefined || turn.stop !== 'error' || isContextLengthError(turn.error)) {
    return -1;
  }
  const actions = turn.error?.actions;
  return actions === undefined || actions.includes('retry') ? view.turns.length - 1 : -1;
}
