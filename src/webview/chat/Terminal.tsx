import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { ChevronDown } from 'lucide-react';
import type { ToolCallBlock } from '@shared/transcript';
import { useScrollFade } from '../ui/useScrollFade';
import { cn } from '../ui/cn';
import { OutputCopy } from './OutputCopy';
import { Prose } from './Prose';
import { looksLikeMarkdown } from './markdownGuess';
import { t } from '../i18n';

// Command output (Codex-style "Shell" card). The row above names the program only; the full command heads the card behind a
// muted `$` when `command` is given, and the output area below sticks to the bottom while running until the user scrolls up
// inside it. Height is capped by --code-output-max.
export function TerminalOutput({ block, command }: { block: ToolCallBlock; command?: string }) {
  const text = outputOf(block);
  if (!text && !command) return null;
  return (
    <div className="group/code-output code-output terminal-surface">
      {/* The copy button belongs to the whole card (top-right corner), not to the output pane under the command */}
      {text && <OutputCopy text={text} label={t('code.copyOutput')} />}
      {command && <CommandPrompt command={command} />}
      {/* Output under the command keeps only a half gap above it */}
      {text && <OutputPane text={text} follow={outputFollows(block)} label={t('code.commandOutput')}
        className={cn('terminal-scroll whitespace-pre', command && 'pt-gap-half')}>
        {/* npm banners are muted; a leading ✓ (vitest indents it) takes the success color */}
        {text.trimEnd().split('\n').map((line, index, lines) => <span key={index} className={/^(?:> |Done in )/.test(line) ? 'text-fg-3' : undefined}>
          {/^\s*✓/.test(line) ? <>{line.slice(0, line.indexOf('✓'))}<span className="text-ok">✓</span>{line.slice(line.indexOf('✓') + 1)}</> : line}{index < lines.length - 1 ? '\n' : ''}
        </span>)}
      </OutputPane>}
    </div>
  );
}

// Text a non-command tool returned (MCP results, background task reports, …) in the command card's frame and padding.
// Unlike terminal output it wraps: these are `key: value` reports and prose, not column-aligned logs, and a horizontal
// scrollbar under a short report cost more than it saved. A wrapped line hangs under its own start
export function ToolOutput({ block, text }: { block: ToolCallBlock; text: string }) {
  const body = text.trimEnd();
  if (!body.trim()) return null;
  // Fetched pages and web search summaries are usually markdown: render them like a message, inside the same capped card.
  // The engine already cuts each text item to TOOL_OUTPUT_MAX, so the rendered document stays bounded
  if (block.kind === 'fetch' && looksLikeMarkdown(body)) return <FetchMarkdown block={block} text={body} />;
  return (
    <div className="group/code-output code-output">
      <OutputPane text={body} follow={outputFollows(block)} label={t('code.toolOutput')} copyLabel={t('code.copyToolOutput')} className="tool-output">
        {body.split('\n').map((line, index) => <span key={index} className="tool-output-line">{line || ' '}</span>)}
      </OutputPane>
    </div>
  );
}

// Markdown body of a fetch tool. The TextBlock is memoized on the text so Prose's memo holds across unrelated re-renders
function FetchMarkdown({ block, text }: { block: ToolCallBlock; text: string }) {
  const doc = useMemo(() => ({ type: 'text' as const, markdown: text }), [text]);
  return (
    <div className="group/code-output code-output">
      <OutputPane text={text} follow={outputFollows(block)} label={t('code.toolOutput')} copyLabel={t('code.copyToolOutput')} prose>
        <Prose block={doc} />
      </OutputPane>
    </div>
  );
}

// Live output sticks to the bottom; an announced call that is only waiting has nothing to follow yet
const outputFollows = (block: ToolCallBlock) =>
  block.observation !== 'unknown' && (block.status === 'in_progress' || block.status === 'pending');

// The scrolling output area both cards share: capped by --code-output-max with edge fades, a copy button on hover when
// `copyLabel` is given (the command card puts its own at the card corner instead), and while `follow` it sticks to the
// bottom until the user scrolls up inside it
// `prose` swaps the monospace <pre> for a block container that hosts rendered markdown
function OutputPane({ text, follow, label, copyLabel, className, prose = false, children }: {
  text: string; follow: boolean; label: string; copyLabel?: string; className?: string; prose?: boolean; children: ReactNode;
}) {
  const ref = useRef<HTMLElement>(null);
  const fade = useScrollFade<HTMLElement>();
  const setRef = useCallback((element: HTMLElement | null) => {
    ref.current = element;
    return fade(element);
  }, [fade]);
  const pinned = useRef(true);
  useEffect(() => {
    const el = ref.current;
    if (el && follow && pinned.current) el.scrollTop = el.scrollHeight;
  }, [text, follow]);
  const Pane = prose ? 'div' : 'pre';
  return (
    <div className="relative min-w-0">
      {copyLabel && <OutputCopy text={text} label={copyLabel} />}
      <Pane
        ref={setRef}
        tabIndex={0}
        aria-label={label}
        onScroll={e => { const el = e.currentTarget; pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24; }}
        className={cn('scroll-fade scroll-thin m-0 max-h-code-output overflow-auto px-command-x py-command-y [overflow-anchor:none]',
          prose ? 'min-w-0' : 'font-mono text-mono text-fg-2', className)}
      >
        {children}
      </Pane>
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
