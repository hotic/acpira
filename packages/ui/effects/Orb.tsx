import { ThinkingOrb } from 'thinking-orbs';
import { ORB_SIZE, ORB_STATE, type OrbKind } from './presets';

// 20px inline Orb, exactly filling the lead slot; theme follows the ancestor data-theme (the library reads it itself).
// Deliberately outside the motion switch: it is the one "still working" signal, a 20px canvas on its own layer, and
// the library already draws a single static frame under the OS reduced-motion preference
export function Orb({ kind, paused }: { kind: OrbKind; paused?: boolean }) {
  return (
    <span className="flex size-lead items-center justify-center">
      <ThinkingOrb state={ORB_STATE[kind]} size={ORB_SIZE} paused={paused} />
    </span>
  );
}
