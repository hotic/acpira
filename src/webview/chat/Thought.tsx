import { memo, useCallback, useEffect, useRef } from 'react';
import { Brain } from 'lucide-react';
import type { ThoughtBlock } from '@shared/transcript';
import { useAppearance } from '../appearance';
import { t } from '../i18n';
import { cn } from '../ui/cn';
import { Disclosure } from '../ui/Disclosure';
import { EntranceOnce } from '../ui/Row';
import { Shimmer } from '../ui/Shimmer';
import { useScrollFade } from '../ui/useScrollFade';
import { StreamText } from './StreamText';
import { useSmoothText } from './streamMotion';

// ACP thought chunks have no end boundary: the next event can arrive only after
// tool arguments finish generating. Keep the text, but never time that gap as thinking.
// The turn heading owns the Orb; thought rows keep a static icon and shimmer only while streaming.
// Models close a thought with blank lines; pre-wrap would render them and push the rail's end dot below the text.
export const Thought = memo(function Thought({ block }: { block: ThoughtBlock }) {
  const { toolLine } = useAppearance();
  const live = !!block.streaming;
  const lead = toolLine === 'text' ? undefined : <Brain className="size-icon" strokeWidth={1.5} />;
  // Thoughts carry no id; the start stamp names them within the turn, so a remounted transcript (another
  // session and back) does not replay the entrance of a thought that already entered
  return (
    <EntranceOnce id={`thought:${block.startedAt ?? ''}`}>
      <Disclosure className="action-details" tone="action" lead={lead} body={<ThoughtBody block={block} />}>
        <Shimmer active={live}>
          {live ? t('host.thinking') : t('thought.label')}
        </Shimmer>
      </Disclosure>
    </EntranceOnce>
  );
});

// The thought text: a bounded scrollport that follows the streamed tail
function ThoughtBody({ block, className }: { block: ThoughtBlock; className?: string }) {
  const fade = useScrollFade<HTMLParagraphElement>();
  const ref = useRef<HTMLParagraphElement>(null);
  const setRef = useCallback((element: HTMLParagraphElement | null) => {
    ref.current = element;
    return fade(element);
  }, [fade]);
  // Past --thought-body-max the text grows inside its own scrollport: follow the tail while streaming, and release once the reader scrolls up inside it (the command output rule)
  const pinned = useRef(true);
  const smooth = useSmoothText(block.text.trimEnd(), !!block.streaming);
  const streaming = !!block.streaming || smooth.draining;
  useEffect(() => {
    const el = ref.current;
    if (el && streaming && pinned.current) el.scrollTop = el.scrollHeight;
  }, [smooth.text, streaming]);
  return <p ref={setRef} onScroll={e => { const el = e.currentTarget; pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24; }}
    className={cn('max-h-(--thought-body-max) overflow-y-auto scroll-fade scroll-thin m-0 text-2 text-fg-2 whitespace-pre-wrap [overflow-wrap:anywhere]', className)}>
    <StreamText text={smooth.text} streaming={streaming} />
  </p>;
}
