import { Fragment, memo, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from 'react';
import { flushSync } from 'react-dom';
import type { AgentId, AgentInfo, ForkOrigin, FailureAction, SlashCommand, Turn, UserTurn } from '@acpira/shared/transcript';
import type { SubagentSummary } from '@acpira/shared/subagents';
import { useAppearance } from '../appearance';
import { useLocale } from '../i18n';
import { cn } from '../ui/cn';
import { useMergedRefs } from '../ui/mergeRefs';
import { QuietEntranceContext } from '../ui/Row';
import { FollowContext, useBottomFollow } from '../chat/useBottomFollow';
import { captureSliceScroll, landSlice, readerScrolling, type SliceScroll } from '../chat/sliceScroll';
import { useExchangeSkip } from '../chat/exchangeSkip';
import { AgentMessage } from '../chat/Turns';
import { HistoryMessage, shownReports } from '../chat/HistoryMessage';
import { planExecutionId } from '@acpira/shared/planExecution';
import { type ThreadSearchBridge } from '../chat/ChatFind';
import { TurnActionsContext } from '../chat/TurnActions';
import { showsTurnActions } from '../chat/turnOutcome';
import { ForkBoundary } from '../chat/ForkBoundary';
import { nodesByTurn } from '../chat/subagents/subagentState';
import { PromptActions } from '../plugins/MessageActions';
import { useSessionOutline } from '../plugins/outline';
import { useReportLink } from '../chat/reportLink';

interface ThreadProps {
  // The session shown, for what plugin frames hear of it (plugins/outline.ts)
  session?: string;
  // The settings' UI / code font sizes: they set the text metrics through CSS variables, not the appearance context
  fontSizes?: string;
  turns: Turn[];
  running: boolean;
  wide: boolean;
  replayKey?: number | string;
  catchingUp?: boolean;
  blobUrl?: (blob: string) => string;
  contentRef?: (el: HTMLDivElement | null) => void;
  // Advertised slash commands: sent user messages paint their `/name` tokens like the composer does
  commands?: SlashCommand[];
  // Enabled subagent persona names: sent user messages paint their `@name` tokens like the composer does
  summons?: readonly string[];
  subagents?: SubagentSummary[];
  // The fork boundary: drawn after the exchange holding the last copied turn
  forkedFrom?: ForkOrigin;
  agents: AgentInfo[];
  agentId: AgentId;
  onInspect?: (id: string) => void;
  onPermission: (blockId: string, optionId: string) => void;
  onFailureAction?: (action: FailureAction) => void;
  // Offers "Set as goal" under the last typed prompt (absent: no goal can be set now)
  // The button is expected once the starting agent reports its goal capability: keep its room meanwhile
  searchBridge?: RefObject<ThreadSearchBridge | undefined>;
}

// Entrance stagger caps out at the 12th block, so long sessions don't take seconds
const STAGGER_CAP = 12;

// Long transcripts mount the tail exchanges first so the viewport paints within one frame on session switch,
// then prepend earlier exchanges in frame slices while keeping the scroll position anchored. A slice is a count of
// exchanges, and in the largest transcripts one exchange can hold hundreds of KB of tool output: 14 at a time made the
// frames after a switch to a 13 MB session take ~45 ms in WebKit, 6 keep them near 25 ms
const EXCHANGE_WINDOW_THRESHOLD = 10;
const INITIAL_EXCHANGES = 8;
const EXCHANGE_SLICE = 6;

function canWindowExchanges(count: number): boolean {
  return count > EXCHANGE_WINDOW_THRESHOLD
    && typeof requestAnimationFrame === 'function'
    && !/jsdom/i.test(globalThis.navigator?.userAgent ?? '');
}

function useExchangeWindow(
  count: number,
  scrollerRef: RefObject<HTMLDivElement | null>,
  follow: { readonly value: boolean },
  opening: { readonly current: boolean },
) {
  const [start, setStart] = useState(() => canWindowExchanges(count) ? count - INITIAL_EXCHANGES : 0);
  const slicing = useRef(false);
  const [quiet] = useState(() => ({ get current() { return opening.current || slicing.current; } }));
  const before = useRef<SliceScroll | undefined>(undefined);

  useEffect(() => {
    if (start <= 0) return;
    let frame2 = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const land = () => {
      const scroller = scrollerRef.current;
      // A scroll the reader has in flight would not survive the offset moving under it: look again next frame
      if (scroller && readerScrolling(scroller)) {
        frame2 = requestAnimationFrame(() => { timer = setTimeout(land, 0); });
        return;
      }
      before.current = scroller ? captureSliceScroll(scroller, follow.value) : undefined;
      slicing.current = true;
      flushSync(() => setStart(cur => Math.max(0, cur - EXCHANGE_SLICE)));
    };
    // Listening from the mount on, so a wheel turned before the first slice holds it too
    if (scrollerRef.current) readerScrolling(scrollerRef.current);
    // Wait until the first frame has painted before prepending earlier slices so the switch paint is never delayed by background mounts.
    const frame1 = requestAnimationFrame(() => {
      frame2 = requestAnimationFrame(() => { timer = setTimeout(land, 0); });
    });
    return () => {
      cancelAnimationFrame(frame1);
      if (frame2) cancelAnimationFrame(frame2);
      if (timer !== undefined) clearTimeout(timer);
    };
  }, [start, scrollerRef, follow]);

  useLayoutEffect(() => {
    slicing.current = false;
    const kept = before.current;
    before.current = undefined;
    if (!kept) return;
    landSlice(kept);
  }, [start]);

  const effectiveStart = start > 0 ? Math.min(start, Math.max(0, count - INITIAL_EXCHANGES)) : 0;
  // Mount every exchange from `k` on now (a plugin's scrollTo an exchange not reached yet)
  const showFrom = useCallback((k: number) => { if (k >= 0) setStart(cur => Math.min(cur, k)); }, []);
  return { start: effectiveStart, quiet, showFrom };
}

// Conversation flow: stick-to-bottom following happens on transcript changes (new content / streaming growth) and on async growth
// (images, diagrams) until the user touches the thread; user actions like expand / collapse never touch the scroll position —
// the toggle under the mouse stays put while the content below it moves. Scrolling away from the bottom releases the follow; scrolling back to the bottom restores it

export const Thread = memo(function Thread({ session, fontSizes, turns, running, wide, replayKey, catchingUp, blobUrl, contentRef, commands, summons, subagents, forkedFrom, agents, agentId, onInspect, onPermission, onFailureAction, searchBridge }: ThreadProps) {
  const locale = useLocale();
  const appearance = useAppearance();
  const ref = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const bodyRef = useMergedRefs(contentRef, body);
  const lastPrompt = useMemo(() => {
    for (let i = turns.length - 1; i >= 0; i--) if (turns[i]!.role === 'user') return turns[i];
    return undefined;
  }, [turns]);
  const follow = useBottomFollow(ref, body, replayKey, [turns, running], lastPrompt);
  // Exchanges out of sight skip style, layout and paint once laid out at their real size (chat/exchangeSkip.ts)
  const exchangeRef = useExchangeSkip(appearance, locale, fontSizes, follow.rewrap);
  // Fade the lower edge while the reader is scrolled away from the bottom (thread-fade): text leaving the view above a
  // borderless dock row (the connecting notice) dissolves instead of being cut off flat. Driven by the follow state rather
  // than useScrollFade, whose content observer would flash the fade for a frame on every streamed chunk before the re-pin
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const sync = () => el.toggleAttribute('data-more-below', !follow.value);
    sync();
    return follow.subscribe(sync);
  }, [follow]);
  // The transcript's first commit restores what the session already holds: its rows appear in place, without entrances.
  // A switch back to a kept session mounts the kept copy first, and the host's catch-up for the time away lands a round
  // trip later: that commit is still quiet. The flag is read during render, before this effect of the same commit runs,
  // so the commit in which `catchingUp` turns false still sees it set; anything after that animates as live output
  const opening = useRef(true);
  useLayoutEffect(() => { opening.current = !!catchingUp; });

  // Group the session's subagent nodes by the turn that announced them; unchanged arrays keep their
  // reference so memoized turns do not re-render on an unrelated child update
  const byTurnRef = useRef<Map<number, SubagentSummary[]> | undefined>(undefined);
  const byTurn = nodesByTurn(subagents ?? [], byTurnRef.current);
  byTurnRef.current = byTurn;

  // Each user message sticks only within its own exchange. Automatic commands belong
  // to the preceding exchange so compaction does not replace the user's context.
  let i = 0;
  // A submitted prompt may already be visible below the active /compact turn.
  let activeAgentIndex = turns.length - 1;
  while (activeAgentIndex >= 0 && turns[activeAgentIndex]?.role !== 'agent') activeAgentIndex--;
  const exchangeSlots: { key: number; items: { ti: number; index: number }[] }[] = [];
  turns.forEach((turn, ti) => {
    const previous = turns[ti - 1];
    if (planExecutionId(turn, previous)) return;
    const index = Math.min(i, STAGGER_CAP);
    i += turn.role === 'agent' ? turn.blocks.length + 1 : 1;
    if (!exchangeSlots.length || (turn.role === 'user' && !turn.auto)) {
      exchangeSlots.push({ key: ti, items: [] });
    }
    exchangeSlots[exchangeSlots.length - 1]!.items.push({ ti, index });
  });
  const window = useExchangeWindow(exchangeSlots.length, ref, follow, opening);
  if (searchBridge) {
    searchBridge.current = { mountAll: () => window.showFrom(0), releaseFollow: () => follow.release() };
  }
  const starts = exchangeSlots.map(e => e.key);
  useSessionOutline(ref, session, turns, starts, running, { mount: turn => window.showFrom(starts.indexOf(turn)), release: () => follow.release() });
  // The context report's hover or pick lights its calls' rows here and dims the rest (chat/reportLink.ts)
  useReportLink(ref, body, { session, enabled: appearance.reportLink !== 'off', turns, starts, showFrom: window.showFrom, release: () => follow.release() });
  // The prompt "Set as goal" hangs under: the last one the user typed, unless it was a slash command
  const canFork = Boolean(useContext(TurnActionsContext)?.fork);
  // The fork boundary follows the exchange that holds the last copied turn, as a row of its own: inside that exchange it
  // would stand after the reply's action row, which the exchange spacing keys off
  const forkAfter = forkedFrom ? exchangeSlots.filter(e => e.key <= forkedFrom.turnIndex).length - 1 : -1;
  // Track explicitly whether each exchange (and the thread tail) ends in TurnActions: WKWebView does not invalidate
  // `:has(> :last-child > [data-turn-actions]:last-child)` when a sibling (such as an auto `/compact` turn) is appended.
  const renderedExchanges = exchangeSlots.slice(window.start).map((slot, offset) => {
    const k = window.start + offset;
    const messages: ReactNode[] = [];
    let endsWithActions = false;
    for (const { ti, index } of slot.items) {
      const turn = turns[ti]!;
      const previous = turns[ti - 1];
      const compacting = previous?.role === 'user' && /^\/compact(?:\s|$)/.test(previous.text.trim());
      // Fold memory key: the session plus the turn's position and start time, so an edited-away turn at the same index does not inherit a choice
      const memoryKey = replayKey !== undefined ? `${replayKey}:${ti}:${turn.role === 'agent' ? turn.startedAt ?? '' : ''}` : undefined;
      // Session-wide nodes only reach turns that announce one; handing the list to every
      // memoized turn would re-render all of them on each child activity tick.
      const mine = turn.role === 'agent' ? byTurn.get(ti) : undefined;
      // An automatic account switch or a Retry continues the cut-off turn: the pair reads as one reply, with the switch
      // notice or the error row as the only seam, so the cut-off half drops its action row and the continuation joins it
      const next = turns[ti + 1];
      // Summoned agents' reports continue the exchange the same way: the reply above, the report rows and the reply they
      // wake read as one answer, so only the woken reply keeps an action row (one in the middle stood as a blank on
      // desktop, where it shows on hover, and as a stray bar on phones)
      const switchedAway = next?.role === 'user' && (isContinue(next) || next.autoReason === 'subagents');
      // The reply woken by summoned agents' reports hugs the one-line report row above it, like a continuation
      const switchedIn = previous?.role === 'user' && (isContinue(previous) || previous.autoReason === 'subagents');
      const afterRows = switchedIn && previous.autoReason === 'subagents' && !isContinue(previous);
      // The engine's own /compact (a hidden prompt) closes the exchange above: its row stands under the reply's action
      // row at a row's pitch, as one of that reply's steps would, instead of a message gap off it (the row floated in a
      // band twice the gap to the next prompt)
      const afterActions = compacting && previous.auto === true && endsWithActions;
      if (turn.role === 'user' && !turn.auto) {
        endsWithActions = false;
        // Key by slot index so an edited prompt stays mounted across its new turn id and finishes its height glide smoothly.
        // Plugin buttons under a typed prompt (the prompt.actions slot); nothing renders without one
        messages.push(
          <HistoryMessage key={ti} turn={turn} turnIndex={ti} index={index} blobUrl={blobUrl} commands={commands} summons={summons} />,
          <PromptActions key={`actions${ti}`} session={session} turn={ti} text={turn.text} />,
        );
        continue;
      }
      const last = ti === turns.length - 1;
      if (turn.role === 'user') {
        // Automatic user turns render nothing unless they carry visible subagent handback rows (HistoryMessage.tsx)
        if (turn.autoReason === 'subagents' && shownReports(turn.text).length > 0) endsWithActions = false;
        messages.push(<HistoryMessage key={ti} turn={turn} turnIndex={ti} index={index} blobUrl={blobUrl} commands={commands} summons={summons} />);
      } else {
        const turnRunning = running && ti === activeAgentIndex && !turn.stop;
        endsWithActions = !switchedAway && !turnRunning && !compacting && showsTurnActions(turn, canFork);
        // Tie the reply key to its preceding prompt's id (stable when a heldPrompt lands, fresh when an edit replaces the turn)
        // so a replacement reply mounts clean instead of inheriting the previous turn's fold refs and delayed CSS transitions
        const agentKey = previous?.role === 'user' && previous.id ? `a:${ti}:${previous.id}` : `a:${ti}:${turn.startedAt ?? ''}`;
        messages.push(<AgentMessage key={agentKey} turn={turn} index={index} compacting={compacting} running={turnRunning} onPermission={onPermission} memoryKey={memoryKey}
          turnIndex={ti} last={last} settings={previous?.role === 'user' ? previous.settings : undefined} actions={!switchedAway} joined={afterRows || afterActions ? 'rows' : switchedIn}
          subagents={mine} allSubagents={mine ? subagents : undefined} onInspect={onInspect} onFailureAction={last ? onFailureAction : undefined} />);
      }
    }
    return { key: slot.key, k, messages, endsWithActions };
  });
  const lastExchange = renderedExchanges.at(-1);
  const tailEndsWithActions = Boolean(lastExchange?.endsWithActions && !(forkedFrom && lastExchange.k === forkAfter));
  // The right padding gives the scrollbar gutter back, so the transcript's edges line up with the header and the dock.
  // Focusable (tabIndex -1) so a press in the transcript takes focus off the composer: VS Code's webview runs ⌘C as
  // execCommand('copy') after refocusing the last focused element whenever the body holds focus (pre/index.html, 1.143),
  // which moved the selection into the composer and copied nothing
  return (
    <div ref={ref} data-thread tabIndex={-1} className="thread-fade scroll-stable min-h-0 min-w-0 flex-1 overflow-y-auto pl-dock pr-dock-gutter outline-none [overflow-anchor:none]">
      {/* The tail clearance equals the message gap, so the last message sits as far from the composer as from the message above it,
          plus the floating connecting row's room when the tail holds no action row (`--thread-tail-room`, chat.css) */}
      <div key={replayKey} ref={bodyRef} data-thread-body data-tail-actions={tailEndsWithActions || undefined} className={cn('pt-(--pad-y) flex flex-col gap-msg pb-[calc(var(--msg-gap)+var(--thread-tail-room))]', wide ? 'thread-column' : 'mx-auto')}><FollowContext.Provider value={follow}><QuietEntranceContext.Provider value={window.quiet}>
        {renderedExchanges.map(({ key, k, messages, endsWithActions }) => (
          <Fragment key={key}>{
            // Positioned so the prompt's stuck-state sentinel can sit at the exchange's top edge. Paint containment gives each exchange
            // its own paint offset, so a fold opening mid-thread no longer re-walks every later exchange each frame (see docs/dev/webview.md,
            // transcript render budget); the clip it brings is pushed out by --hit on the left and bottom, where row hit areas and
            // card shadows reach past the column, and on the right only as far as the thread's right padding (--hit-end), and the top edge stays put for the sentinel. An exchange ending in a reply's action
            // row pulls the next one up until the icons stand as far from the next prompt as from the reply text (TurnActions,
            // --spacing-turn-actions-below). It reads --msg-gap itself: the --spacing-* aliases resolve where the theme declares
            // them, above the element that carries the density, so they keep the default density's value.
            <section ref={exchangeRef} data-exchange data-exchange-start={key} data-ends-actions={endsWithActions || undefined} className="relative -ml-hit -mr-hit-end -mb-hit flex min-w-0 flex-col gap-msg pl-hit pr-hit-end pb-hit contain-paint data-[ends-actions]:-mb-[calc(var(--spacing-hit)+var(--msg-gap)-var(--spacing-turn-actions-below))]">
              {messages}
            </section>
          }{k === forkAfter && forkedFrom && <ForkBoundary origin={forkedFrom} agents={agents} agentId={agentId} />}</Fragment>
        ))}
      </QuietEntranceContext.Provider></FollowContext.Provider></div>
    </div>
  );
});

// A hidden turn that carries the turn before it on (after an account switch, Retry after a failure with output, the
// workspace gate sending its findings back, an active goal's next turn, or the sidekick's reply alone coming back: its
// relay's handback row already shows the return)
export function isContinue(turn: UserTurn): boolean {
  return turn.auto === true && (turn.autoReason === 'accountSwitch' || turn.autoReason === 'retry' || turn.autoReason === 'gate' || (turn.autoReason === 'goal' && turn.goalContinue === true)
    || (turn.autoReason === 'subagents' && shownReports(turn.text).length === 0));
}
