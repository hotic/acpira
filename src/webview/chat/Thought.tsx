import { memo, useCallback, useEffect, useRef } from 'react';
import { Brain } from 'lucide-react';
import type { ThoughtBlock } from '@shared/transcript';
import { useAppearance } from '../appearance';
import { t } from '../i18n';
import { Disclosure } from '../ui/Disclosure';
import { Shimmer } from '../ui/Shimmer';
import { useScrollFade } from '../ui/useScrollFade';
import { StreamText } from './StreamText';

// ACP thought chunks have no end boundary: the next event can arrive only after
// tool arguments finish generating. Keep the text, but never time that gap as thinking.
// The turn heading owns the Orb; thought rows keep a static icon and shimmer only while streaming.
// Models close a thought with blank lines; pre-wrap would render them and push the rail's end dot below the text.
export const Thought = memo(function Thought({ block }: { block: ThoughtBlock }) {
  const { toolLine } = useAppearance();
  const fade = useScrollFade<HTMLParagraphElement>();
  const ref = useRef<HTMLParagraphElement>(null);
  const setRef = useCallback((element: HTMLParagraphElement | null) => {
    ref.current = element;
    return fade(element);
  }, [fade]);
  // Past --thought-body-max the text grows inside its own scrollport: follow the tail while streaming, and release once the reader scrolls up inside it (the command output rule)
  const pinned = useRef(true);
  useEffect(() => {
    const el = ref.current;
    if (el && block.streaming && pinned.current) el.scrollTop = el.scrollHeight;
  }, [block.text, block.streaming]);
  const lead = toolLine === 'text' ? undefined : <Brain className="size-icon" strokeWidth={1.5} />;
  return (
    <Disclosure className="action-details" tone="action" lead={lead} body={<p ref={setRef} onScroll={e => { const el = e.currentTarget; pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24; }} className="max-h-(--thought-body-max) overflow-y-auto scroll-fade scroll-thin m-0 text-2 text-fg-2 whitespace-pre-wrap [overflow-wrap:anywhere]"><StreamText text={block.text.trimEnd()} streaming={block.streaming} /></p>}>
      <Shimmer active={!!block.streaming}>
        {block.streaming ? t('host.thinking') : t('thought.label')}
      </Shimmer>
    </Disclosure>
  );
});
