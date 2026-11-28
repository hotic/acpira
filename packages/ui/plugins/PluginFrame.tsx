import { cn } from '../ui/cn';
import { usePluginPiece } from '../pluginSlots';
import { SandboxFrame } from './SandboxFrame';

export { digest, tokensOf } from './pluginFrameDoc';

// One interface piece of a plugin in the tool panel (`panel.tab`): rendered in a sandboxed frame without
// allow-same-origin. A piece no longer in effect (switched off, safe mode) leaves nothing
export function PluginFrame({ plugin, piece, shown, className }: { plugin: string; piece: string; shown: boolean; className?: string }) {
  const found = usePluginPiece(plugin, piece);
  if (found?.html === undefined) return null;
  return (
    <SandboxFrame plugin={plugin} piece={piece} title={found.title || plugin} focusable={shown}
      aria-hidden={!shown || undefined}
      className={cn('absolute inset-0 size-full', !shown && 'hidden', className)} />
  );
}
