import { useCallback, useLayoutEffect, useState, type RefObject } from 'react';
import { capToChat, fitPaneBounds, usePreferredPaneWidth, type PaneBounds } from '../chat/subagents/InspectorSplitter';

// Whether the subagent inspector docks beside the chat column, and the docked pane's width: the user's (drag its left
// edge), clamped by CSS so the conversation keeps --subagent-main-min
export function useInspectorDock(mainColumn: RefObject<HTMLDivElement | null>, inspectorPane: RefObject<HTMLElement | null>) {
  const [canDockInspector, setCanDockInspector] = useState(false);
  const [preferredPane, setPreferredPane] = usePreferredPaneWidth();
  const [draggedPane, setDraggedPane] = useState<number>();
  useLayoutEffect(() => {
    const element = mainColumn.current;
    if (!element) return;
    const minWidth = Number.parseFloat(getComputedStyle(element).getPropertyValue('--subagent-dock-min'));
    // Hysteresis: once docked, the column lost the pane's width — count it back so the panel does not flap open/closed
    const update = (entries?: ResizeObserverEntry[]) => {
      const w = entries?.[0]?.contentRect.width ?? element.clientWidth;
      const room = w + (inspectorPane.current?.offsetWidth ?? 0);
      setCanDockInspector(room >= minWidth);
    };
    update();
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const paneBounds = useCallback((): PaneBounds => {
    const styles = getComputedStyle(mainColumn.current ?? document.documentElement);
    const px = (name: string) => Number.parseFloat(styles.getPropertyValue(name));
    const room = (mainColumn.current?.clientWidth ?? 0) + (inspectorPane.current?.offsetWidth ?? 0);
    const bounds = fitPaneBounds(inspectorPane.current, { min: px('--subagent-pane-min'), max: room - px('--subagent-main-min'), initial: px('--subagent-pane-w') });
    // Its left edge borders the chat: widening takes from the chat, never from the tool panel right of it
    return capToChat(bounds, inspectorPane.current, mainColumn.current);
  }, []);
  const paneWidth = draggedPane ?? preferredPane;
  return { canDockInspector, paneWidth, paneBounds, setDraggedPane, setPreferredPane };
}
