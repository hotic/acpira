import { useContext, useState, type ComponentProps } from 'react';
import { Popover as Base } from '@base-ui/react/popover';
import { ShellLayerContext, overlayWidth, popupClass, useOpenLifecycle, useShellPosition, type OverlayWidth } from './Overlay';
import { cn, cnState } from './cn';
export { ShellLayerContext } from './Overlay';

function Root({ open: controlled, defaultOpen = false, onOpenChange, onOpenLifecycle, ...props }: ComponentProps<typeof Base.Root> & { onOpenLifecycle?: (open: boolean) => void }) {
  const [inner, setInner] = useState(defaultOpen);
  const open = controlled ?? inner;
  useOpenLifecycle(open, onOpenLifecycle);
  return <Base.Root {...props} modal={false} open={open} onOpenChange={(next, details) => {
    onOpenChange?.(next, details);
    if (!details.isCanceled) setInner(next);
  }} />;
}
function Portal(props: Omit<ComponentProps<typeof Base.Portal>, 'container'>) {
  const layer = useContext(ShellLayerContext);
  return layer ? <Base.Portal {...props} container={layer} /> : null;
}
function Positioner({ width = 'md', className, side = 'bottom', align = 'start', sideOffset, collisionPadding, collisionBoundary, collisionAvoidance, ...props }: ComponentProps<typeof Base.Positioner> & { width?: OverlayWidth | 'anchor' }) {
  const { shell, gap, pad, width: shellWidth } = useShellPosition();
  return <Base.Positioner side={side} align={align} sideOffset={sideOffset ?? gap}
    collisionBoundary={collisionBoundary ?? shell} collisionPadding={collisionPadding ?? pad}
    collisionAvoidance={collisionAvoidance ?? { side: 'flip', align: 'shift' }}
    {...props} className={cnState(cn('z-30', width === 'anchor' ? 'w-(--anchor-width)' : overlayWidth[width]), className)}
    style={state => ({ maxWidth: shellWidth ? shellWidth - pad * 2 : undefined, ...(typeof props.style === 'function' ? props.style(state) : props.style) })} />;
}
function Popup({ className, initialFocus = false, palette = 'widget', ...props }: ComponentProps<typeof Base.Popup> & { palette?: 'widget' | 'menu' | 'suggest' }) {
  return <Base.Popup initialFocus={initialFocus} data-palette={palette} {...props} className={cnState(cn(popupClass), className)} />;
}
export const Popover = { Root, Trigger: Base.Trigger, Portal, Positioner, Popup, Close: Base.Close, Title: Base.Title, Description: Base.Description };
