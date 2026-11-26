import { describe, expect, it, vi } from 'vitest';
import { createScrimDismiss } from './Dialog';

describe('createScrimDismiss', () => {
  const scrim = { id: 'scrim' };
  const input = { id: 'input' };

  it('dismisses when a primary press starts and ends on the scrim', () => {
    const onDismiss = vi.fn();
    const h = createScrimDismiss(onDismiss);
    h.onPointerDownCapture({ target: scrim, currentTarget: scrim, button: 0 });
    h.onPointerUpCapture({ target: scrim, currentTarget: scrim });
    h.onClick({ target: scrim, currentTarget: scrim });
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it('does not dismiss when a press starts inside the dialog and releases on the scrim', () => {
    const onDismiss = vi.fn();
    const h = createScrimDismiss(onDismiss);
    // Browser synthesises click on the common ancestor (the scrim) after dragging out of an input
    h.onPointerDownCapture({ target: input, currentTarget: scrim, button: 0 });
    h.onPointerUpCapture({ target: scrim, currentTarget: scrim });
    h.onClick({ target: scrim, currentTarget: scrim });
    expect(onDismiss).not.toHaveBeenCalled();
  });

  it('does not dismiss when a press starts on the scrim and releases inside the dialog', () => {
    const onDismiss = vi.fn();
    const h = createScrimDismiss(onDismiss);
    h.onPointerDownCapture({ target: scrim, currentTarget: scrim, button: 0 });
    h.onPointerUpCapture({ target: input, currentTarget: scrim });
    h.onClick({ target: scrim, currentTarget: scrim });
    expect(onDismiss).not.toHaveBeenCalled();
  });

  it('does not dismiss after a previous aborted scrim press followed by an inside drag-out', () => {
    const onDismiss = vi.fn();
    const h = createScrimDismiss(onDismiss);
    // Aborted scrim press (e.g. released outside the window with no pointerup/click)
    h.onPointerDownCapture({ target: scrim, currentTarget: scrim, button: 0 });
    // Subsequent drag from inside the dialog back out onto the scrim
    h.onPointerDownCapture({ target: input, currentTarget: scrim, button: 0 });
    h.onPointerUpCapture({ target: scrim, currentTarget: scrim });
    h.onClick({ target: scrim, currentTarget: scrim });
    expect(onDismiss).not.toHaveBeenCalled();
  });

  it('ignores non-primary buttons, cancelled pointers and prevented clicks', () => {
    const onDismiss = vi.fn();
    const h = createScrimDismiss(onDismiss);

    h.onPointerDownCapture({ target: scrim, currentTarget: scrim, button: 2 });
    h.onPointerUpCapture({ target: scrim, currentTarget: scrim });
    h.onClick({ target: scrim, currentTarget: scrim });

    h.onPointerDownCapture({ target: scrim, currentTarget: scrim, button: 0 });
    h.onPointerCancelCapture();
    h.onClick({ target: scrim, currentTarget: scrim });

    h.onPointerDownCapture({ target: scrim, currentTarget: scrim, button: 0 });
    h.onPointerUpCapture({ target: scrim, currentTarget: scrim });
    h.onClick({ target: scrim, currentTarget: scrim, defaultPrevented: true });

    expect(onDismiss).not.toHaveBeenCalled();
  });
});
