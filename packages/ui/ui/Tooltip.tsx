import { useContext, type ComponentProps } from 'react';
import { Tooltip as Base } from '@base-ui/react/tooltip';
import { ShellLayerContext, overlayWidth, popupClass, useShellPosition, type OverlayWidth } from './Overlay';
import { cn, cnState } from './cn';

function Portal(props: Omit<ComponentProps<typeof Base.Portal>, 'container'>) {
  const layer = useContext(ShellLayerContext);
  return <Base.Portal {...props} container={layer ?? undefined} />;
}

function Positioner({ width = 'sm', className, side = 'top', align = 'start', sideOffset, collisionPadding, collisionBoundary, collisionAvoidance, ...props }: ComponentProps<typeof Base.Positioner> & { width?: OverlayWidth | 'auto' }) {
  const { shell, gap, pad, width: shellWidth } = useShellPosition();
  return (
    <Base.Positioner
      side={side}
      align={align}
      sideOffset={sideOffset ?? gap}
      collisionBoundary={collisionBoundary ?? shell}
      collisionPadding={collisionPadding ?? pad}
      collisionAvoidance={collisionAvoidance ?? { side: 'flip', align: 'shift' }}
      {...props}
      className={cnState(cn('z-30', width !== 'auto' && overlayWidth[width]), className)}
      style={state => ({
        maxWidth: shellWidth ? shellWidth - pad * 2 : undefined,
        ...(typeof props.style === 'function' ? props.style(state) : props.style),
      })}
    />
  );
}

function Popup({ className, palette = 'widget', ...props }: ComponentProps<typeof Base.Popup> & { palette?: 'widget' | 'menu' | 'suggest' }) {
  return <Base.Popup data-palette={palette} {...props} className={cnState(cn(popupClass), className)} />;
}

export const Tooltip = {
  Provider: Base.Provider,
  Root: Base.Root,
  Trigger: Base.Trigger,
  Portal,
  Positioner,
  Popup,
};
