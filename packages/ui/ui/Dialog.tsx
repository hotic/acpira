import { useContext, useRef, type ComponentProps } from 'react';
import { Dialog as Base } from '@base-ui/react/dialog';
import { ShellLayerContext } from './Overlay';

function Root({ modal = false, ...props }: ComponentProps<typeof Base.Root>) {
  return <Base.Root {...props} modal={modal} />;
}
function Portal(props: Omit<ComponentProps<typeof Base.Portal>, 'container'>) {
  const layer = useContext(ShellLayerContext);
  return layer ? <Base.Portal {...props} container={layer} /> : null;
}

// Dismiss only when a primary press both starts and ends on the scrim itself: dragging a selection out of a dialog's
// field or card onto the scrim (or past the window edge) still fires `click` on the scrim as the common ancestor.
export function createScrimDismiss(onDismiss: () => void) {
  let armed = false;
  return {
    onPointerDownCapture(e: { target: unknown; currentTarget: unknown; button: number }) {
      armed = e.target === e.currentTarget && e.button === 0;
    },
    onPointerUpCapture(e: { target: unknown; currentTarget: unknown }) {
      if (e.target !== e.currentTarget) armed = false;
    },
    onPointerCancelCapture() {
      armed = false;
    },
    onClick(e: { target: unknown; currentTarget: unknown; defaultPrevented?: boolean }) {
      const ok = armed && e.target === e.currentTarget;
      armed = false;
      if (ok && !e.defaultPrevented) onDismiss();
    },
  };
}

function Popup({
  onScrimClick,
  onPointerDownCapture,
  onPointerUpCapture,
  onPointerCancelCapture,
  onClick,
  ...props
}: ComponentProps<typeof Base.Popup> & { onScrimClick?: () => void }) {
  const onScrimClickRef = useRef(onScrimClick);
  onScrimClickRef.current = onScrimClick;
  const scrim = useRef<ReturnType<typeof createScrimDismiss> | null>(null);
  scrim.current ??= createScrimDismiss(() => onScrimClickRef.current?.());
  return (
    <Base.Popup
      initialFocus={false}
      finalFocus={false}
      {...props}
      onPointerDownCapture={e => {
        scrim.current!.onPointerDownCapture(e);
        onPointerDownCapture?.(e);
      }}
      onPointerUpCapture={e => {
        scrim.current!.onPointerUpCapture(e);
        onPointerUpCapture?.(e);
      }}
      onPointerCancelCapture={e => {
        scrim.current!.onPointerCancelCapture();
        onPointerCancelCapture?.(e);
      }}
      onClick={e => {
        onClick?.(e);
        scrim.current!.onClick(e);
      }}
    />
  );
}
export const Dialog = { Root, Portal, Popup, Close: Base.Close, Title: Base.Title, Description: Base.Description };
