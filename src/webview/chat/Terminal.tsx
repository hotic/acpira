import { useCallback, useEffect, useRef } from 'react';
import type { ToolCallBlock } from '@shared/transcript';
import { useScrollFade } from '../ui/useScrollFade';
import { OutputCopy } from './OutputCopy';
import { t } from '../i18n';

// Command output (Codex-style "Shell" card): the command itself lives on the tool row above; this is only the output area,
// sticking to the bottom while running and stopping once the user scrolls up inside it. Height is capped by --code-output-max.
export function TerminalOutput({ block }: { block: ToolCallBlock }) {
  const text = outputOf(block);
  const follow = block.status === 'in_progress' || block.status === 'pending';
  const ref = useRef<HTMLPreElement>(null);
  const fade = useScrollFade<HTMLPreElement>();
  const setRef = useCallback((element: HTMLPreElement | null) => {
    ref.current = element;
    return fade(element);
  }, [fade]);
  const pinned = useRef(true);
  useEffect(() => {
    const el = ref.current;
    if (el && follow && pinned.current) el.scrollTop = el.scrollHeight;
  }, [text, follow]);
  if (!text) return null;
  return (
    <div className="group/code-output code-output terminal-surface">
      <OutputCopy text={text} label={t('code.copyOutput')} />
      <pre
        ref={setRef}
        tabIndex={0}
        aria-label={t('code.commandOutput')}
        onScroll={e => { const el = e.currentTarget; pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24; }}
        className="terminal-scroll scroll-fade scroll-thin m-0 max-h-code-output overflow-auto whitespace-pre p-pad font-mono text-mono leading-code-output text-fg-2 [overflow-anchor:none]"
      >
        {text.trimEnd().split('\n').map((line, index, lines) => <span key={index} className={/^(?:> |Done in )/.test(line) ? 'text-fg-3' : undefined}>
          {line.startsWith('✓') ? <><span className="text-ok">✓</span>{line.slice(1)}</> : line}{index < lines.length - 1 ? '\n' : ''}
        </span>)}
      </pre>
    </div>
  );
}

function outputOf(b: ToolCallBlock): string {
  const c = b.content;
  if (!c) return '';
  if (c.type === 'text') return c.text;
  if (c.type === 'list') return c.items.join('\n');
  return c.lines.map(l => l.text).join('\n');
}
