import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';

// Outlasts the list page's 320 ms opening (desktop.css): a transition is cut short once its property leaves `transition`
const DRAWER_MOVE_MS = 450;

// A narrow window's session list as a page over the chat: open / closed, and the window in which it slides
export function useDrawer(desktop: boolean, canDock: boolean, position: string, root: RefObject<HTMLDivElement | null>, sessionPanel: RefObject<HTMLElement | null>) {
  const [drawerOpen, setDrawerOpen] = useState(false);
  const drawerOpenNow = useRef(drawerOpen);
  drawerOpenNow.current = drawerOpen;
  // Desktop: a narrow window's list page slides only while the user opens or closes it (desktop.css data-drawer-moving),
  // never as the window crosses the width tier. A counter, so a second move restarts the clock instead of cutting it short
  const [drawerMoving, setDrawerMoving] = useState(0);
  useEffect(() => {
    if (!drawerMoving) return;
    const timer = setTimeout(() => setDrawerMoving(0), DRAWER_MOVE_MS);
    return () => clearTimeout(timer);
  }, [drawerMoving]);
  const openDrawer = useCallback(() => {
    if (drawerOpenNow.current) return;
    setDrawerMoving(m => m + 1);
    setDrawerOpen(true);
  }, []);
  // `slide`: the user closed it; the width tier closes it in place
  const closeDrawer = useCallback((slide = true) => {
    if (!drawerOpenNow.current) return;
    if (sessionPanel.current?.contains(document.activeElement)) {
      root.current?.querySelector<HTMLButtonElement>('[data-session-toggle]')?.focus();
    }
    if (slide) setDrawerMoving(m => m + 1);
    setDrawerOpen(false);
  }, []);
  // A tier crossing (or a moved list) closes it in place and ends the motion window: a crossing back within it would slide
  // the page out again
  useEffect(() => {
    closeDrawer(false);
    setDrawerMoving(0);
  }, [position, canDock, closeDrawer]);
  // Desktop: Esc closes a narrow window's list page wherever the focus is (inside it, the list's own handler takes the key
  // first; an open menu stops it before it gets here). The key is consumed: Electron hands a key the page leaves alone to
  // the macOS menu bar, where a synthetic Esc crashed the app in the input method code (the smoke, 2026-10-23)
  useEffect(() => {
    if (!desktop || canDock || !drawerOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented || e.isComposing) return;
      e.preventDefault();
      closeDrawer();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [desktop, canDock, drawerOpen, closeDrawer]);
  useEffect(() => {
    if (drawerOpen && !canDock) sessionPanel.current?.querySelector<HTMLInputElement>('input')?.focus();
  }, [drawerOpen, canDock]);
  const toggleDrawer = useCallback(() => (drawerOpenNow.current ? closeDrawer() : openDrawer()), [closeDrawer, openDrawer]);
  return { drawerOpen, drawerMoving, openDrawer, closeDrawer, toggleDrawer };
}
