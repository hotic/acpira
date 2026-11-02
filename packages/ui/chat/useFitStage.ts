import { useLayoutEffect, useRef, useState } from 'react';

// Lets the rest of the toolbar give way before the item steps past `stage`: below that form's need the hook calls `set(true)`
// (the toolbar folds its other chips to their icons) and keeps the item at `stage` or fuller while folded. It unfolds once
// the room the folded chips gave back is no longer needed, judged against their unfolded width, so the two never chase each other
export interface FitSqueeze {
  stage: number;
  set: (on: boolean) => void;
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
  // Whether the toolbar is folded for this item, and what the rest of the toolbar took up just before it folded
  const folded = useRef<{ others: number } | undefined>(undefined);
  useLayoutEffect(() => {
    const unfold = () => {
      if (!folded.current) return;
      folded.current = undefined;
      squeezeRef.current?.set(false);
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
        const others = bar.clientWidth - parseFloat(barStyle.paddingLeft) - parseFloat(barStyle.paddingRight) - row.offsetWidth;
        if (!folded.current) {
          // Hold the protected form while the fold lands; the measure after it settles the real stage
          if (next > sq.stage) { folded.current = { others }; sq.set(true); next = sq.stage; }
        } else if (room - (folded.current.others - others) >= needs[sq.stage]! + 2) {
          // The room this row would have with the others unfolded again covers the form the fold protects
          unfold();
        } else next = Math.max(next, sq.stage); // folded: never back past the protected form, so the steps only go one way
      }
      setStage(next);
    };
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
    return () => { resize.disconnect(); children.disconnect(); unfold(); };
  }, [enabled]);
  return { stage, item, box };
}
