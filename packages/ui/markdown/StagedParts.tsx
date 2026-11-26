import { memo, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import type { TextBlock } from '@acpira/shared/transcript';
import { Prose } from './Prose';
import { previewParts } from './previewParts';

// YAML front matter (a skill's `name` / `description`) would render as a rule and a setext heading; it shows as a YAML block
function previewText(text: string): string {
  const front = /^---\r?\n([\s\S]*?)\r?\n---[ \t]*(?:\r?\n|$)/.exec(text);
  return front ? `\`\`\`yaml\n${front[1]}\n\`\`\`\n\n${text.slice(front[0].length)}` : text;
}

// `render` must be a stable function (module level): a part renders again only when its text changes
const StagedPart = memo(function StagedPart({ text, render, part }: { text: string; render: (text: string) => ReactNode; part?: boolean }) {
  // Memoized so the `seen` flip below does not render (and parse) the content again
  const content = useMemo(() => render(text), [render, text]);
  const ref = useRef<HTMLDivElement>(null);
  // Once laid out at its real height, a part out of sight skips style, layout and paint (panel.css) and keeps that
  // height, so the document's stays exact. Skipped from the start, it would stand at an estimate until scrolled to
  const [seen, setSeen] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!part || seen || !el) return;
    // The first observation comes after the part's first layout, when its size is also remembered
    const observer = new ResizeObserver(() => { observer.disconnect(); setSeen(true); });
    observer.observe(el);
    return () => observer.disconnect();
  }, [part, seen]);
  // prose.css gives a heading that opens a later part the gap it has in the whole
  return part ? <div ref={ref} data-preview-part data-seen={seen || undefined}>{content}</div> : content;
});

// A long document shown in parts mounted a frame apart from the top (the Markdown preview here, TextPeek's raw and
// rendered text), so the first screen shows at once and no frame lays out the whole. A new text keeps the parts already
// shown, so a live edit neither shrinks the document under its scroll nor renders it all in one frame; an unchanged
// part does not render again. `className` is the multi-part column's
export function StagedParts({ parts, render, className }: { parts: string[]; render: (text: string) => ReactNode; className?: string }) {
  const [shown, setShown] = useState(1);
  useEffect(() => {
    if (shown >= parts.length) return;
    const frame = requestAnimationFrame(() => setShown(shown + 1));
    return () => cancelAnimationFrame(frame);
  }, [shown, parts.length]);
  if (parts.length === 1) return <StagedPart text={parts[0]!} render={render} />;
  return <div className={className}>{parts.slice(0, shown).map((text, i) => <StagedPart key={i} text={text} render={render} part />)}</div>;
}

const proseOf = (markdown: string) => <Prose block={{ type: 'text', markdown } satisfies TextBlock} />;

// Markdown rendered the way a reply is, in parts (previewParts.ts) for a long document
export function MarkdownParts({ text }: { text: string }) {
  const parts = useMemo(() => previewParts(previewText(text)), [text]);
  return <StagedParts parts={parts} render={proseOf} className="flex min-w-0 flex-col gap-gap" />;
}
