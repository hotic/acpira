import { useCallback } from 'react';

// Fade only edges with hidden content. Observe text updates and disclosure sizing,
// so streaming, resizing and mounting an initially empty output update immediately.
export function useScrollFade<T extends HTMLElement>() {
  return useCallback((element: T | null) => {
    if (!element) return;
    let frame: number | undefined;
    const update = () => {
      if (element.clientHeight < 1) return;
      const overflow = element.scrollHeight - element.clientHeight;
      element.toggleAttribute('data-more-above', overflow > 1 && element.scrollTop > 1);
      element.toggleAttribute('data-more-below', overflow > 1 && overflow - element.scrollTop > 1);
    };
    const schedule = () => {
      if (typeof requestAnimationFrame === 'undefined') { update(); return; }
      if (frame !== undefined) return;
      frame = requestAnimationFrame(() => { frame = undefined; update(); });
    };
    update();
    element.addEventListener('scroll', schedule, { passive: true });
    const size = new ResizeObserver(schedule);
    size.observe(element);
    const content = new MutationObserver(schedule);
    content.observe(element, { childList: true, characterData: true, subtree: true });
    return () => {
      element.removeEventListener('scroll', schedule);
      size.disconnect();
      content.disconnect();
      if (frame !== undefined) cancelAnimationFrame(frame);
    };
  }, []);
}
