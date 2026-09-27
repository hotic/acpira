import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ChevronDown } from 'lucide-react';
import type { ToolCallBlock } from '@shared/transcript';
import { useScrollFade } from '../ui/useScrollFade';
import { cn } from '../ui/cn';
import { OutputCopy } from './OutputCopy';
import { t } from '../i18n';

// Command output (Codex-style "Shell" card). The row above names the program only; the full command heads the card behind a
// muted `$` when `command` is given, and the output area below sticks to the bottom while running until the user scrolls up
// inside it. Height is capped by --code-output-max.
export function TerminalOutput({ block, command }: { block: ToolCallBlock; command?: string }) {
  const text = outputOf(block);
  const follow = block.observation !== 'unknown' && (block.status === 'in_progress' || block.status === 'pending');
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
  if (!text && !command) return null;
  return (
    <div className="group/code-output code-output terminal-surface">
      {command && <CommandPrompt command={command} />}
      {text && <div className="relative min-w-0">
        <OutputCopy text={text} label={t('code.copyOutput')} />
        <pre
          ref={setRef}
          tabIndex={0}
          aria-label={t('code.commandOutput')}
          onScroll={e => { const el = e.currentTarget; pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24; }}
          className={cn('terminal-scroll scroll-fade scroll-thin m-0 max-h-code-output overflow-auto whitespace-pre px-command-x py-command-y font-mono text-mono text-fg-2 [overflow-anchor:none]',
            // Output under the command keeps only a half gap above it
            command && 'pt-gap-half')}
        >
          {/* npm banners are muted; a leading ✓ (vitest indents it) takes the success color */}
          {text.trimEnd().split('\n').map((line, index, lines) => <span key={index} className={/^(?:> |Done in )/.test(line) ? 'text-fg-3' : undefined}>
            {/^\s*✓/.test(line) ? <>{line.slice(0, line.indexOf('✓'))}<span className="text-ok">✓</span>{line.slice(line.indexOf('✓') + 1)}</> : line}{index < lines.length - 1 ? '\n' : ''}
          </span>)}
        </pre>
      </div>}
    </div>
  );
}

// The full command, cut to --command-prompt-lines until expanded. Wrapped and heredoc lines hang under the command, not the `$`.
// One chevron follows the text in both states (down while cut, up once open); the cut point is measured so text and chevron
// share the last visible line. Height animates between the two states (off under motion=none / reduced motion).
function CommandPrompt({ command }: { command: string }) {
  const [expanded, setExpanded] = useState(false);
  // Characters shown while collapsed; the whole command when it fits
  const [cut, setCut] = useState(command.length);
  const ref = useRef<HTMLDivElement>(null);
  const probe = useRef<HTMLDivElement>(null);
  const from = useRef<number | null>(null);
  useLayoutEffect(() => {
    const wrap = ref.current?.parentElement;
    const el = probe.current;
    if (!wrap || !el) return;
    let width = -1;
    const measure = () => {
      if (wrap.clientWidth === width) return;
      width = wrap.clientWidth;
      setCut(fitCommand(el, command));
    };
    measure();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(measure);
    observer.observe(wrap);
    return () => observer.disconnect();
  }, [command]);
  // Runs after the text swapped: grow or shrink from the height it had before the toggle
  useLayoutEffect(() => {
    const el = ref.current;
    const start = from.current;
    from.current = null;
    if (!el || start === null || !Number.parseFloat(getComputedStyle(el).transitionDuration)) return;
    // Natural height first: under a fixed height scrollHeight never reports less than that height
    el.style.height = '';
    const end = el.scrollHeight;
    el.style.height = `${start}px`;
    void el.offsetHeight;
    el.style.height = `${end}px`;
  }, [expanded]);
  const toggle = () => {
    from.current = ref.current?.offsetHeight ?? null;
    setExpanded(!expanded);
  };
  const clipped = cut < command.length;
  const shown = expanded || !clipped ? command : command.slice(0, cut).trimEnd() + (command[cut] === '\n' ? '' : '…');
  const label = expanded ? t('code.commandCollapse') : t('code.commandExpand');
  return (
    <div className="command-prompt px-command-x pt-command-y font-mono text-mono text-fg-3 last:pb-command-y">
      <div className="relative min-w-0">
        <div ref={ref} className="command-prompt-text"
          onTransitionEnd={e => { if (e.target === e.currentTarget) e.currentTarget.style.height = ''; }}>
          <span className="select-none" aria-hidden="true">$ </span>{shown}
          {clipped && <button type="button" aria-expanded={expanded} title={label} aria-label={label} onClick={toggle}
            className="command-prompt-toggle ml-gap-half inline-flex align-middle">
            <ChevronDown className="command-prompt-chevron size-3" strokeWidth={1.75} data-open={expanded || undefined} />
          </button>}
        </div>
        <div ref={probe} className="command-prompt-text command-prompt-probe" aria-hidden="true" />
      </div>
    </div>
  );
}

// Longest prefix of `command` that, followed by an ellipsis and the chevron, stays within --command-prompt-lines on the
// probe (an invisible copy of the text box at the same width). Returns the full length when the whole command fits.
function fitCommand(probe: HTMLElement, command: string): number {
  const style = getComputedStyle(probe);
  const line = Number.parseFloat(style.lineHeight);
  const limit = Number.parseInt(style.getPropertyValue('--command-prompt-lines'), 10);
  if (!(line > 0) || !Number.isFinite(limit)) return command.length;
  const text = document.createTextNode('');
  const tail = document.createElement('span');
  tail.className = 'command-prompt-toggle ml-gap-half inline-flex align-middle';
  tail.innerHTML = '<span class="inline-block size-3"></span>';
  probe.replaceChildren('$ ', text, tail);
  const fits = (length: number, suffix: string) => {
    text.data = command.slice(0, length) + suffix;
    return Math.round(probe.offsetHeight / line) <= limit;
  };
  tail.hidden = true;
  const whole = fits(command.length, '');
  tail.hidden = false;
  if (whole) return probe.replaceChildren(), command.length;
  let low = 0;
  let high = command.length;
  while (low < high) {
    const mid = Math.ceil((low + high) / 2);
    if (fits(mid, '…')) low = mid;
    else high = mid - 1;
  }
  probe.replaceChildren();
  return low;
}

function outputOf(b: ToolCallBlock): string {
  // Several text items in wire order render as one output stream
  if (b.contents?.length) {
    const texts = b.contents.filter((c): c is Extract<typeof c, { type: 'text' }> => c.type === 'text');
    if (texts.length) return texts.map(c => c.text).join('\n');
  }
  const c = b.content;
  if (!c) return '';
  if (c.type === 'text') return c.text;
  if (c.type === 'list') return c.items.join('\n');
  if (c.type === 'diff') return c.lines.map(l => l.text).join('\n');
  return '';
}
