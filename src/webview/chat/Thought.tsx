import { memo, useCallback, useEffect, useMemo, useRef } from 'react';
import { Brain } from 'lucide-react';
import type { TextBlock, ThoughtBlock } from '@shared/transcript';
import { useAppearance } from '../appearance';
import { t } from '../i18n';
import { cn } from '../ui/cn';
import { Disclosure } from '../ui/Disclosure';
import { EntranceOnce } from '../ui/Row';
import { Shimmer } from '../ui/Shimmer';
import { useScrollFade } from '../ui/useScrollFade';
import { Prose } from './Prose';
import { useAutoFold } from './autoFold';

// ACP thought chunks have no end boundary: the next event can arrive only after
// tool arguments finish generating. Keep the text, but never time that gap as thinking.
// The turn heading owns the Orb; thought rows keep a static icon and shimmer only while streaming.
export const Thought = memo(function Thought({ block }: { block: ThoughtBlock }) {
  const { toolLine } = useAppearance();
  const live = !!block.streaming;
  const lead = toolLine === 'text' ? undefined : <Brain className="size-icon" strokeWidth={1.5} />;
  // Thoughts carry no id; the start stamp names them within the turn, so a remounted transcript (another
  // session and back) does not replay the entrance of a thought that already entered
  const fold = useAutoFold();
  return (
    <EntranceOnce id={`thought:${block.startedAt ?? ''}`}>
      <Disclosure className="action-details" tone="action" lead={lead} open={fold?.open} onToggle={fold?.onToggle} body={<ThoughtBody block={block} />}>
        <Shimmer active={live}>
          {live ? t('host.thinking') : t('thought.label')}
        </Shimmer>
      </Disclosure>
    </EntranceOnce>
  );
});

// The thought text: Markdown through the reply renderer (GPT's reasoning summaries open each section with
// `**Title**` and use lists / inline code), inside a bounded scrollport that follows the streamed tail.
// Models close a thought with blank lines; trimming them keeps the rail's end dot level with the last line.
function ThoughtBody({ block, className }: { block: ThoughtBlock; className?: string }) {
  const fade = useScrollFade<HTMLDivElement>();
  const ref = useRef<HTMLDivElement>(null);
  const setRef = useCallback((element: HTMLDivElement | null) => {
    ref.current = element;
    return fade(element);
  }, [fade]);
  // Prose memoizes on the block reference: rebuild it only when the thought itself changed
  const text = useMemo<TextBlock>(() => ({ type: 'text', markdown: block.text.trimEnd(), streaming: block.streaming }), [block.text, block.streaming]);
  // Past --thought-body-max the text grows inside its own scrollport: follow the tail while the text is still
  // being drawn, and release once the reader scrolls up inside it (the command output rule)
  const pinned = useRef(true);
  const busy = useRef(!!block.streaming);
  const onBusy = useCallback((value: boolean) => { busy.current = value; }, []);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    // Prose paces the text inside its own renders (and mounts nothing until the first characters), so follow
    // the rendered DOM rather than this component's props
    const observer = new MutationObserver(() => {
      if (busy.current && pinned.current) el.scrollTop = el.scrollHeight;
    });
    observer.observe(el, { childList: true, characterData: true, subtree: true });
    return () => observer.disconnect();
  }, []);
  return <div ref={setRef} onScroll={e => { const el = e.currentTarget; pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24; }}
    className={cn('max-h-(--thought-body-max) overflow-y-auto scroll-fade scroll-thin', className)}>
    <Prose block={text} tone="thought" onBusy={onBusy} />
  </div>;
}
