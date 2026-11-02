import type { ToolCallBlock } from '@acpira/shared/transcript';
import { RowLabel } from '../ui/Row';
import { toolVerb } from './folding';

// The heading's text and shimmer share the phase status, including gaps between same-kind calls. Individual
// results stay completed; the label keeps its natural width and the row supplies the gap before the target.
export function ToolLabel({ block, continuing = false }: { block: ToolCallBlock; continuing?: boolean }) {
  const status = continuing && block.status === 'completed' ? 'in_progress' : block.status;
  return <RowLabel shimmer={status === 'in_progress' && block.observation !== 'unknown'} syncShimmer>
    {toolVerb(block, status)}
  </RowLabel>;
}
