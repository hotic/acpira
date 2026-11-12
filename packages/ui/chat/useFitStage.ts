import { useLayoutEffect, useRef, useState } from 'react';

// How far the toolbar gives way for the item: 1 folds its other chips to their icons, 2 (`tight`) also hides its optional
// icon chips (the context ring, summon, live context, plugin buttons) once even the item's last form no longer fits
export type SqueezeLevel = 0 | 1 | 2;

// Lets the rest of the toolbar give way before the item steps past `stage`: below that form's need the hook calls `set(1)`
// and keeps the item at `stage` or fuller while folded; below the last form's need it calls `set(2)` and holds the last form.
// Each level lifts once the room the hidden parts gave back is no longer needed, judged against what the bar took before
// that level landed, so the two never chase each other
export interface FitSqueeze {
  stage: number;
  set: (level: SqueezeLevel) => void;
}

// Picks the fullest form of a toolbar item that fits the room its flex row leaves it. Every form is laid out once in a hidden
// measuring box (`box`, one child per form, fullest first, inside the same container so the container queries match); the
// room is the row's width less its other in-flow items, so it never depends on the form shown and a stage change cannot
// feed back into the next measure. Flexible items (the toolbar's spacer) count as free room. Shrunk siblings are counted at
// their content width. Without a box the stage stays 0
export function useFitStage<I extends HTMLElement, B extends HTMLElement>(enabled: boolean, squeeze?: FitSqueeze) {
  const item = useRef<I | null>(null);
  const box = useRef<B | null>(null);
  const [stage, setStage] = useState(0);
  const squeezeRef = useRef(squeeze);
  squeezeRef.current = squeeze;
  // Whether the toolbar is folded for this item, and what the rest of the toolbar took up just before it folded; `tight`:
  // what the bar took besides the item's room just before the optional chips went too
  const folded = useRef<{ others: number; tight?: number } | undefined>(undefined);
  // The level last asked for. A level that lands without moving anything (below the sm tier the container queries have
  // already folded the other chips) fires no resize, which left the item on the form held for the fold, running under the
  // send button; the commit that carries the level measures again instead
  const [level, setLevel] = useState<SqueezeLevel>(0);
  const measure = useRef<(() => void) | undefined>(undefined);
  useLayoutEffect(() => { measure.current?.(); }, [level]);
  useLayoutEffect(() => {
    const ask = (next: SqueezeLevel) => { squeezeRef.current?.set(next); setLevel(next); };
    const unfold = () => {
      if (!folded.current) return;
      folded.current = undefined;
      ask(0);
    };
    if (!enabled) { setStage(0); unfold(); return; }
    const el = item.current, forms = box.current, row = el?.parentElement;
    if (!el || !forms || !row) return;
    const bar = row.parentElement;
    const update = () => {
      const style = getComputedStyle(row);
      let used = 0, count = 0;
      for (const child of row.children) {
        const s = getComputedStyle(child);
        if (s.position === 'absolute' || s.position === 'fixed' || s.display === 'none') continue;
        count++;
        if (child === el || parseFloat(s.flexGrow) > 0) continue;
        used += Math.max((child as HTMLElement).offsetWidth, child.scrollWidth);
      }
      const room = row.clientWidth - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight) - used - (parseFloat(style.columnGap) || 0) * Math.max(0, count - 1);
      const needs = [...forms.children].map(form => (form as HTMLElement).offsetWidth);
      const fit = needs.findIndex(need => need <= room + 0.5);
      let next = fit < 0 ? needs.length - 1 : fit;
      const sq = squeezeRef.current;
      if (sq && bar && sq.stage < needs.length) {
        // What the bar's other items take: the bar's inner width less this row
        const barStyle = getComputedStyle(bar);
        const inner = bar.clientWidth - parseFloat(barStyle.paddingLeft) - parseFloat(barStyle.paddingRight);
        const others = inner - row.offsetWidth;
        const last = needs.length - 1;
        if (!folded.current) {
          // Hold the protected form while the fold lands; the measure after it settles the real stage
          if (next > sq.stage) { folded.current = { others }; ask(1); next = sq.stage; }
        } else if (folded.current.tight !== undefined) {
          // Tight: the optional chips come back once the room left without them again covers the last form
          if (inner - folded.current.tight >= needs[last]! + 2) { folded.current.tight = undefined; ask(1); }
          next = last;
        } else if (fit < 0) {
          // Folded and still short of the last form: the marks would run under the send button, so the optional chips go
          folded.current.tight = inner - room;
          ask(2);
          next = last;
        } else if (room - (folded.current.others - others) >= needs[sq.stage]! + 2) {
          // The room this row would have with the others unfolded again covers the form the fold protects
          unfold();
        } else next = Math.max(next, sq.stage); // folded: never back past the protected form, so the steps only go one way
      }
      setStage(next);
    };
    measure.current = update;
    const resize = new ResizeObserver(update);
    const watch = () => {
      resize.disconnect();
      for (const target of [row, forms, ...row.children, ...forms.children]) resize.observe(target);
      update();
    };
    // Siblings come and go (the context ring, an added model): re-observe the row's children on every change
    const children = new MutationObserver(watch);
    children.observe(row, { childList: true });
    watch();
    return () => { measure.current = undefined; resize.disconnect(); children.disconnect(); unfold(); };
  }, [enabled]);
  return { stage, item, box };
}
