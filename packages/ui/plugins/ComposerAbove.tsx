import { memo, useMemo } from 'react';
import { usePluginPieces } from '../pluginSlots';
import { PieceFrame, versionKey } from './Pieces';
import type { FrameState } from './SandboxFrame';
import { backdropsShown } from './skin';

// The composer.above slot (docs/dev/plugins.md Interface): plugin pieces sitting on the top edge of the dock under the
// conversation (the plan card, a question, the queue or the composer, whichever is highest), at its end, at the box
// their plugin declares. They take no room and no clicks (styles/skin.css), and hear only whether the session works or
// waits for the user (`acpira.onState`). Plain flags in, so a streamed chunk does not render it. Desktop only, where
// the frame document is served
export const ComposerAbove = memo(function ComposerAbove({ running, waiting }: { running: boolean; waiting: boolean }) {
  const pieces = usePluginPieces('composer.above');
  const state = useMemo<FrameState>(() => ({ working: running && !waiting, waiting }), [running, waiting]);
  if (!backdropsShown || !pieces.length) return null;
  return (
    <div data-plugin-above aria-hidden="true">
      <div>{pieces.map(p => <PieceFrame key={versionKey(p)} piece={p} state={state} hidden />)}</div>
    </div>
  );
});
