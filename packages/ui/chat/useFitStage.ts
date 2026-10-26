import { useLayoutEffect, useRef, useState } from 'react';

// Picks the fullest form of a toolbar item that fits the room its flex row leaves it. Every form is laid out once in a hidden
// measuring box (`box`, one child per form, fullest first, inside the same container so the container queries match); the
// room is the row's width less its other in-flow items, so it never depends on the form shown and a stage change cannot
// feed back into the next measure. Flexible items (the toolbar's spacer) count as free room. Shrunk siblings are counted at
// their content width. Without a box the stage stays 0
export function useFitStage<I extends HTMLElement, B extends HTMLElement>(enabled: boolean) {
  const item = useRef<I | null>(null);
  const box = useRef<B | null>(null);
  const [stage, setStage] = useState(0);
  useLayoutEffect(() => {
    if (!enabled) { setStage(0); return; }
    const el = item.current, forms = box.current, row = el?.parentElement;
    if (!el || !forms || !row) return;
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
      setStage(fit < 0 ? needs.length - 1 : fit);
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
    return () => { resize.disconnect(); children.disconnect(); };
  }, [enabled]);
  return { stage, item, box };
}
