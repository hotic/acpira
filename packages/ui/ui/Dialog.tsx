import { useContext, type ComponentProps } from 'react';
import { Dialog as Base } from '@base-ui/react/dialog';
import { ShellLayerContext } from './Overlay';

function Root({ modal = false, ...props }: ComponentProps<typeof Base.Root>) {
  return <Base.Root {...props} modal={modal} />;
}
function Portal(props: Omit<ComponentProps<typeof Base.Portal>, 'container'>) {
  const layer = useContext(ShellLayerContext);
  return layer ? <Base.Portal {...props} container={layer} /> : null;
}
function Popup(props: ComponentProps<typeof Base.Popup>) {
  return <Base.Popup initialFocus={false} finalFocus={false} {...props} />;
}
export const Dialog = { Root, Portal, Popup, Close: Base.Close, Title: Base.Title, Description: Base.Description };
