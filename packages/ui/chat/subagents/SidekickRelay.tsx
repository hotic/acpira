import { createContext, useContext, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { SIDEKICK_ID, type SubagentSummary } from '@acpira/shared/subagents';
import type { ConfigControl, ToolCallBlock } from '@acpira/shared/transcript';
import { composerControls, effortLabel } from '@acpira/shared/composerControls';
import { brandColor, brandLayers, modelBrand, optionBrand } from '@acpira/shared/models';
import { t, useLocale } from '../../i18n';
import { ChipTag } from '../../ui/Button';
import { EntranceScopeContext, QuietEntranceContext, Row, RowEntranceContext, RowLabel, RowTarget, pageHidden, useEntranceEnd } from '../../ui/Row';
import { cn } from '../../ui/cn';
import { cssTimeVar } from '../../ui/cssTime';
import { ModelMark } from '../ModelMark';
import { commandDuration } from '../commandDuration';
import { CommandOutcome } from '../CommandOutcome';
import { toolTarget, toolVerb } from '../folding';
import { useToolSeconds } from '../useToolSeconds';
import { isWaiting, resultText, stateLabel, subagentTitle } from './subagentState';
import { useElapsed } from './useElapsed';

// A round of the built-in agent's sidekick: the lead's own second hand, not a summoned helper
export const isSidekick = (n: SubagentSummary) => n.harness?.persona === SIDEKICK_ID;

// The lead model and the sidekick's display names, from the session's controls (chat/Shell.tsx). The relay's first mark is
// the lead's, and a round carries only the sidekick select's value
export interface SidekickModels { lead?: { name: string; brand?: string }; names: ReadonlyMap<string, string> }
export const SidekickModelsContext = createContext<SidekickModels>({ names: new Map() });

// Rows the window holds; the engine sends as many steps (`SIDEKICK_STEPS`)
const WINDOW = 4;

// Whether what happens right now happens out of the reader's sight: the first commit of a thread or of a slice (restored
// history), or a hidden page (the same gates `Row` applies to its own entrance). Read at each event, so a round mounted
// quietly still slides in the steps and the handback that arrive in front of the reader
function useStill(): () => boolean {
  const quiet = useContext(QuietEntranceContext);
  return () => quiet.current || pageHidden();
}

// Rounds whose handoff already played, and rounds seen running (their window keeps every slot). Both outlive a remount,
// which a long thread's tail window or a group changing shape causes mid-round
const handedOff = new Set<string>();
const reserved = new Set<string>();
const LEDGER_CAP = 4000;
function remember(set: Set<string>, key: string) {
  if (set.size >= LEDGER_CAP) set.clear();
  set.add(key);
}

// The keyframes each entrance waits on before its flag drops
const HEAD_PARTS = ['sidekick-flip', 'sidekick-sheen', 'sidekick-slide', 'acp-fade-in'];
const STEP_PARTS = ['sidekick-slide'];
const BACK_PARTS = ['sidekick-pop', 'sidekick-sheen', 'sidekick-slide', 'acp-fade-in'];

// The sidekick's part of one delegation: the handoff row, a fixed-height window streaming its latest steps, and the
// handback line taking the window's last slot. Nothing here changes height while the round runs or ends
export function SidekickRelay({ node, onInspect }: { node: SubagentSummary; onInspect: (id: string) => void }) {
  useLocale();
  const { lead, names } = useContext(SidekickModelsContext);
  const still = useStill();
  const { scope } = useContext(EntranceScopeContext);
  const key = `${scope}\u0000${node.id}`;
  const running = node.state === 'running';
  const model = node.model ? names.get(node.model) ?? node.model : t('subagents.untitled');
  const brand = (node.model && modelBrand(node.model)) ?? modelBrand(model);
  const mark = <ModelMark family={model} brand={brand} colored />;
  const title = subagentTitle(node, t);
  const elapsed = useElapsed(node);
  const waiting = isWaiting(node);
  const status = waiting ? (node.permissions?.length ? t('host.awaitingApproval') : t('host.awaitingAnswers')) : running ? stateLabel(node, t) : undefined;

  // The handoff whooshes once per round, only when it happens on screen; the handback when the round ends on screen
  const [handoff, setHandoff] = useState(() => {
    // Claimed by the first mount whatever it shows, so a round first met in restored history never hands off later
    const first = !handedOff.has(key);
    remember(handedOff, key);
    return first && running && !still();
  });
  const head = useRef<HTMLButtonElement>(null);
  useEntranceEnd(handoff, head, HEAD_PARTS, () => setHandoff(false));
  const wasRunning = useRef(running);
  const [handback, setHandback] = useState(false);
  useLayoutEffect(() => {
    if (wasRunning.current && !running && !still()) setHandback(true);
    wasRunning.current = running;
  }, [running]);

  // A round seen running keeps the whole window for good, so its end never shrinks the block under the reader; a settled
  // one sizes to what it has. The handback is the list's last item
  const [reserve] = useState(() => {
    if (running) remember(reserved, key);
    return reserved.has(key);
  });
  const kept = useKept(node.steps);
  const items: (ToolCallBlock | undefined)[] = running ? kept : [...kept, undefined];
  const slots = reserve ? WINDOW : Math.max(1, Math.min(WINDOW, items.length));
  // Steps already there when the block mounted stay still; later ones slide in from the gutter
  const [seen] = useState(() => new Set(kept.map(s => s.id)));
  const list = useSlide(items.map(s => s?.id ?? '\u0001back').join('\u0000'), slots, still);

  // The block runs its own entrances; Row's staggered part fades would put the words ahead of the light
  return (
    <RowEntranceContext.Provider value={false}><div className="sidekick-relay" style={brandStyle(brand)} aria-label={t('sidekick.relay', { model })}>
      <Row
        as="button"
        interactive
        ref={head}
        className="sidekick-head"
        data-enter={handoff ? '' : undefined}
        title={node.task ?? title}
        aria-label={`${t('sidekick.relay', { model })} ${t('sidekick.takes')} ${title}${status ? ` ${status}` : ''} ${elapsed}`}
        onClick={() => onInspect(node.id)}
        lead={<Flip from={lead && <ModelMark family={lead.name} brand={lead.brand} />} to={mark} />}
        trailing={<>
          {node.harness?.effort && <span className="sidekick-level flex"><ChipTag small>{effortLabel({ id: node.harness.effort, name: node.harness.effort })}</ChipTag></span>}
          <span className="sidekick-clock whitespace-nowrap">
            {status && <span className={cn(waiting ? 'text-warn' : 'text-fg-3')}>{status} </span>}{elapsed}
          </span>
        </>}
      >
        <span className="sidekick-sheen" aria-hidden />
        {/* Under room pressure the task gives up its width first, then the model name; the verb always stays */}
        <span className="sidekick-label flex min-w-0 items-baseline gap-gap overflow-hidden">
          <span className="sidekick-name min-w-0 truncate text-fg-1">{model}</span>
          <span className="shrink-0 text-fg-2">{t('sidekick.takes')}</span>
          {(node.harness?.round ?? 1) > 1 && <span className="shrink-0 text-3 text-fg-3">{t('subagents.round', { n: node.harness!.round })}</span>}
          <span className="min-w-0 truncate text-fg-3 [flex-shrink:1000]">{title}</span>
        </span>
      </Row>
      <div className="sidekick-window" data-more={items.length > slots ? '' : undefined} style={{ '--sidekick-slots': slots } as CSSProperties}>
        <div ref={list} className="sidekick-window-list">
          {items.map(s => s
            ? <Step key={s.id} block={s} mark={mark} enter={!seen.has(s.id)} still={still} />
            : <Handback key="back" node={node} mark={mark} steps={node.toolCount} elapsed={elapsed} enter={handback}
                onEntered={() => setHandback(false)} onInspect={onInspect} />)}
        </div>
      </div>
    </div></RowEntranceContext.Provider>
  );
}

// How much of each ink a colour logo's handoff light carries, and where across the band the inks sit
const SHEEN_INK = 14;
const SHEEN_SPAN = [20, 80];

// The sidekick takes its vendor's colour where the vendor has one (subagents.css `--sidekick`). A colour logo's handoff
// light carries all its inks, faint, in the order the logo lays them left to right, its base last
export function brandStyle(brand: string | undefined): CSSProperties | undefined {
  const color = brandColor(brand);
  if (!color) return undefined;
  const inks = [...brandLayers(brand) ?? []].sort((a, b) => a.from[0] - b.from[0]).map(l => l.color);
  if (!inks.length) return { '--sidekick-brand': color } as CSSProperties;
  inks.push(color);
  const [from, to] = SHEEN_SPAN;
  const stops = inks.map((c, i) => `color-mix(in srgb, ${c} ${SHEEN_INK}%, transparent) ${from! + (to! - from!) * i / (inks.length - 1)}%`);
  return { '--sidekick-brand': color, '--sidekick-sheen': `linear-gradient(90deg, transparent, ${stops.join(', ')}, transparent)` } as CSSProperties;
}

// The engine sends the newest few steps; the one it just dropped stays a moment longer, so the slide that brings a new
// step in moves real rows instead of opening a gap at the top
function useKept(steps: ToolCallBlock[] | undefined): ToolCallBlock[] {
  const kept = useRef<ToolCallBlock[]>([]);
  return useMemo(() => {
    const now = steps ?? [];
    const ids = new Set(now.map(s => s.id));
    const next = [...kept.current.filter(s => !ids.has(s.id)), ...now].slice(-(WINDOW + 1));
    kept.current = next;
    return next;
  }, [steps]);
}

// The window is anchored at its bottom once it overflows, so a row appended there pushes the rest up at once; this plays
// that push back as a slide of one row per new row. WAAPI, so it checks the motion settings itself
function useSlide(key: string, slots: number, still: () => boolean) {
  const list = useRef<HTMLDivElement>(null);
  const prev = useRef(key ? key.split('\u0000') : []);
  useLayoutEffect(() => {
    const ids = key ? key.split('\u0000') : [];
    const before = prev.current;
    prev.current = ids;
    const el = list.current;
    const last = before.at(-1);
    const added = last === undefined ? ids.length : ids.length - 1 - ids.lastIndexOf(last);
    const rise = Math.min(Math.max(0, added), Math.max(0, ids.length - slots));
    if (!el || still() || rise <= 0 || el.children.length < 2) return;
    if (el.closest('[data-motion="none"]') || window.matchMedia?.('(prefers-reduced-motion: reduce)').matches) return;
    const pitch = (el.children[1] as HTMLElement).offsetTop - (el.children[0] as HTMLElement).offsetTop;
    // A push still under way carries on from where it stands, so quick steps never jump the rows back
    const under = el.getAnimations().length ? new DOMMatrixReadOnly(getComputedStyle(el).transform).m42 : 0;
    el.getAnimations().forEach(a => a.cancel());
    el.animate([{ transform: `translateY(${rise * pitch + under}px)` }, { transform: 'none' }], { duration: cssTimeVar(el, '--sidekick-slide', 440), easing: 'cubic-bezier(0.3, 1.25, 0.5, 1)' });
  }, [key, slots]);
  return list;
}

// The handoff mark: the lead's on the front face, the sidekick's on the back; at rest the back shows
function Flip({ from, to }: { from: ReactNode; to: ReactNode }) {
  return <span className="sidekick-flip" aria-hidden>
    <span className="sidekick-flip-inner">
      <span className="sidekick-flip-face sidekick-flip-front">{from ?? to}</span>
      <span className="sidekick-flip-face sidekick-flip-back">{to}</span>
    </span>
  </span>;
}

// One of the sidekick's tool rows, its mark in the lead slot. Rows stay plain: the inspector holds the outputs
function Step({ block, mark, enter: on, still }: { block: ToolCallBlock; mark: ReactNode; enter: boolean; still: () => boolean }) {
  const [enter, setEnter] = useState(() => on && !still());
  const row = useRef<HTMLDivElement>(null);
  useEntranceEnd(enter, row, STEP_PARTS, () => setEnter(false));
  const running = block.status === 'in_progress' && block.observation !== 'unknown';
  const seconds = useToolSeconds(block);
  const execute = block.kind === 'execute';
  const duration = seconds === undefined ? undefined
    : block.startedAt !== undefined && block.endedAt !== undefined ? commandDuration(block.endedAt - block.startedAt)
      : commandDuration(seconds * 1000, true);
  const target = toolTarget(block);
  // A read names its file: the window is narrow, and the path's head is the same for every step
  const shown = block.kind === 'read' && target ? target.split(/[\\/]/).pop() : target;
  return (
    <Row ref={row} className="sidekick-step" data-enter={enter ? '' : undefined}
      lead={<span className="sidekick-mark">{mark}</span>}
      trailing={execute ? <CommandOutcome status={block.status} duration={duration} />
        : block.status === 'failed' ? <CommandOutcome status={block.status} /> : undefined}>
      <RowLabel shimmer={running}>{toolVerb(block)}</RowLabel>
      {shown && <RowTarget mono={block.targetMono}>{shown}</RowTarget>}
    </Row>
  );
}

// The round's end in the window's last slot: what came back, or how it stopped. Mark, words and light enter on one beat
function Handback({ node, mark, steps, elapsed, enter, onEntered, onInspect }: {
  node: SubagentSummary; mark: ReactNode; steps: number; elapsed: string; enter: boolean; onEntered: () => void; onInspect: (id: string) => void;
}) {
  const ok = node.state === 'completed';
  const row = useRef<HTMLButtonElement>(null);
  useEntranceEnd(enter, row, BACK_PARTS, onEntered);
  const line = node.result && resultText(node.result).split('\n').map(s => s.trim()).find(s => s.length > 0);
  return (
    <Row ref={row} as="button" interactive className="sidekick-back" data-enter={enter ? '' : undefined}
      title={line || undefined}
      onClick={() => onInspect(node.id)}
      lead={<span className="sidekick-mark">{mark}</span>}
      trailing={<span className="sidekick-back-trail whitespace-nowrap">{t('sidekick.steps', { n: steps })} {elapsed}</span>}>
      <span className="sidekick-sheen" aria-hidden />
      <span className="sidekick-back-text flex min-w-0 items-baseline gap-gap">
        <span className={cn('shrink-0', ok ? 'text-fg-2' : node.state === 'failed' ? 'text-danger' : 'text-fg-3')}>{ok ? t('sidekick.handback') : stateLabel(node, t)}</span>
        {line && <span className="min-w-0 truncate text-fg-1">{line}</span>}
      </span>
    </Row>
  );
}

// The relay's model context for one session, kept by value: a push that changes neither the lead nor the sidekick list
// hands the thread the same object, so no memoized turn re-renders for it
export function useSidekickModels(options: ConfigControl[]): SidekickModels {
  const { models, sidekick } = composerControls(options);
  const control = models[0];
  const picked = control?.options.find(o => o.id === control.value);
  const lead = picked ? { name: picked.name, brand: optionBrand(picked) } : undefined;
  const list = sidekick[0]?.options ?? [];
  const key = [lead?.name, lead?.brand, ...list.flatMap(o => [o.id, o.name])].join('\u0000');
  // `key` spells out everything the value reads
  return useMemo(() => ({ lead, names: new Map(list.map(o => [o.id, o.name])) }), [key]);
}
