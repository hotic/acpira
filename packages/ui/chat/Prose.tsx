import { cloneElement, createContext, isValidElement, memo, useContext, useEffect, useLayoutEffect, useState, type ComponentProps, type ReactElement, type ReactNode } from 'react';
import { Block, Streamdown, defaultRehypePlugins, parseMarkdownIntoBlocks, type BlockProps, type Components, type ExtraProps } from 'streamdown';
import { cjk } from '@streamdown/cjk';
import { createMathPlugin } from '@streamdown/math';
import type { ImageRef, TextBlock } from '@acpira/shared/transcript';
import { STREAM_DURATION_MS, STREAM_MOTION, useSmoothText, useStreamMotion, type StreamMotion } from './streamMotion';
import { applyGlyphRuns, createGlyphRuns, seedGlyphRuns, type GlyphRunState } from './glyphRuns';
import { CodeBlock } from './CodeBlock';
import { InlineFileCode, Link } from './Link';
import { decodeLocalImageSrc, rewriteFileHrefs, sameImageSource } from './fileLinks';
import { AgentImage } from './AgentImage';
import { mermaidDiagram } from './mermaidPlugin';
import { useTheme } from '../look';
import { t, useLocale } from '../i18n';
import { Bell } from 'lucide-react';
import { QuietEntranceContext, Row, RowLabel, RowTarget } from '../ui/Row';
import { TASK_NOTE_TAG, isolateTaskNotes, rewriteTaskNotes, withTaskNotes } from './taskNotes';

// Full Markdown via streamdown: GFM + KaTeX + Mermaid, streaming-aware (remend repairs incomplete syntax mid-stream).
// All typography lives in styles/prose.css under .acp-prose — Tailwind never scans node_modules, so streamdown's own classes don't resolve here.

// singleDollarTextMath stays off: "$5 and $10" in prose must not become math.
// cjk relaxes CommonMark's flanking rules so `**…。**范围` still closes the bold run when punctuation meets a CJK letter
const PLUGINS = { cjk, math: createMathPlugin(), mermaid: mermaidDiagram };
// Module-level: streamdown's top-level memo compares props by reference, so every config object must be stable
const LINK_SAFETY = { enabled: false };
const { raw: rehypeRaw, sanitize: rehypeSanitize, harden: rehypeHarden } = defaultRehypePlugins;
if (!rehypeRaw || !rehypeSanitize || !rehypeHarden) throw new Error('streamdown default rehype plugins missing');
// Rewrite file:// before sanitize/harden; urlTransform runs too late and harden would paint ` [blocked]`.
const REHYPE = [rehypeRaw, rewriteFileHrefs, rewriteTaskNotes, withTaskNotes(rehypeSanitize), rehypeHarden];
const REPLY_CLASS = 'acp-prose flex min-w-0 flex-col gap-gap text-1 text-fg-1';
const THOUGHT_CLASS = 'acp-prose acp-thought flex min-w-0 flex-col gap-gap text-2 text-fg-2';
// Static mode renders the whole markdown at once without splitting into blocks; passing a single-element splitter skips
// streamdown's unconditional marked.lex pass on settled turns.
const STATIC_BLOCKS = (markdown: string) => [markdown];
// `motion` is a stable module-level config (streamdown compares props by reference); the LAB passes alternatives.
// `onBusy` hears whether the text is still being drawn (received, paced out or fading in), for a turn that waits for
// its reply to finish on screen before folding its process away; pass a stable function.
// `tone="thought"` renders reasoning text: the secondary type scale and colour, with headings kept at body size (prose.css `.acp-thought`)
export const Prose = memo(function Prose({ block, motion = STREAM_MOTION, onBusy, tone = 'reply' }: { block: TextBlock; motion?: StreamMotion; onBusy?: (busy: boolean) => void; tone?: 'reply' | 'thought' }) {
  // memo keeps a row from rendering again; the locale context still reaches it after a language change
  useLocale();
  // A block still streaming when its thread mounts quietly (the session reopened mid-turn) was already on screen:
  // without this every return to a session whose turn is held open (Claude waiting on background subagents)
  // replayed its last reply from the first character
  const quiet = useContext(QuietEntranceContext);
  const [resume] = useState(() => !!block.streaming && quiet.current);
  // Text that grows in a quiet commit (switching back to a kept session, then the host's catch-up for the time away)
  // was written while the reader looked elsewhere: it shows at once instead of being paced out again. A resumed block
  // gets this too, since the catch-up lands after its mount
  const caughtUp = quiet.current;
  const smooth = useSmoothText(block.markdown, !!block.streaming, motion.pace, resume, caughtUp);
  // Still draining counts as streaming: the renderer keeps its streaming mode until the visible text catches up
  const streaming = !!block.streaming || smooth.draining;
  const { animated, animating } = useStreamMotion(streaming, motion);
  const busy = streaming || animating;
  const [blocks] = useState(() => createLiveBlocks(resume));
  blocks.runs = runsFit(motion);
  if (caughtUp) blocks.resume = true;
  // Only a resumed or caught-up render seeds the block plugins; later text animates past what it rendered
  useLayoutEffect(() => { blocks.resume = false; });
  useEffect(() => { onBusy?.(busy); }, [busy, onBusy]);
  useEffect(() => () => onBusy?.(false), [onBusy]);
  // The turn heading already indicates waiting before the first visible words.
  if (!smooth.text.trim()) return null;
  return (
    <MarkdownImages.Provider value={block.images}>
    <LiveBlocks.Provider value={blocks}>
    <Streamdown
      mode={streaming || animating ? 'streaming' : 'static'}
      isAnimating={streaming || animating}
      animated={animated}
      controls={false}
      lineNumbers={false}
      codeBlockMaxHeight={0}
      tableMaxHeight={0}
      linkSafety={LINK_SAFETY}
      rehypePlugins={REHYPE}
      plugins={PLUGINS}
      components={COMPONENTS}
      BlockComponent={SettlingBlock}
      parseMarkdownIntoBlocksFn={busy ? blocks.parse : STATIC_BLOCKS}
      className={tone === 'thought' ? THOUGHT_CLASS : REPLY_CLASS}
    >
      {isolateTaskNotes(smooth.text)}
    </Streamdown>
    </LiveBlocks.Provider>
    </MarkdownImages.Provider>
  );
});

// streamdown's animate plugin wraps every character of an animated block in its own span and keeps a plugin per
// block, so while a turn streams every block it has ever animated stays one span per character: a minute of
// Claude thinking left ~14k spans in the live thought, and each 32 ms reveal tick paid style, pre-paint,
// compositing-input and layout work over all of them (the main thread saturated and the renderer grew to GBs).
// Only the streaming tail needs the entrance; a block two or more behind it has long finished fading and renders
// as plain text again. The block just before the tail keeps its spans so its last glyphs finish their fade.
const ANIMATED_TAIL_BLOCKS = 2;

// `resume` holds for the first render of a resumed block (see Prose): its text is already on screen, so the tail
// blocks' animate plugins start past it instead of fading every glyph in again
// `runs`: the entrance is drawn by glyphRuns.ts instead of streamdown's per-character spans (the production motion; the
// LAB's alternative entrances with a stagger or another animation keep streamdown's plugin)
interface LiveBlocksState { count: number; resume: boolean; runs: boolean; parse: (markdown: string) => string[] }

const runsFit = (motion: StreamMotion) => motion.animation.animation === 'acpGlyph' && !motion.animation.stagger;

// Per Prose instance: streamdown memoizes its block split on this function's identity, and the blocks render
// right after the split in the same pass, so they read the current count from it
function createLiveBlocks(resume: boolean): LiveBlocksState {
  const state: LiveBlocksState = {
    count: 0,
    resume,
    runs: true,
    parse: markdown => {
      const blocks = parseMarkdownIntoBlocks(markdown);
      state.count = blocks.length;
      return blocks;
    },
  };
  return state;
}

const LiveBlocks = createContext<LiveBlocksState | undefined>(undefined);

// One stripped plugin list per animated list, so a settled block keeps a stable `rehypePlugins` and Block's memo holds
const settledPlugins = new WeakMap<object, BlockProps['rehypePlugins']>();

// A live block's glyph runs and the plugin list that draws them, one per streamdown animate plugin (it keeps one per
// block), so the list stays stable across passes and Block's memo holds while the content does
const runPlugins = new WeakMap<object, { state: GlyphRunState; list: BlockProps['rehypePlugins'] }>();
// A run is drawn as a fading span until its fade has surely ended (the CSS duration, --stream-char-duration, plus a frame or two)
const RUN_FADE_MS = STREAM_DURATION_MS + 40;
// Gives each block's glyph run plugin its own name (see glyphRunPlugins)
let glyphRunPluginSeq = 0;

function glyphRunPlugins(animatePlugin: NonNullable<BlockProps['animatePlugin']>, rehypePlugins: NonNullable<BlockProps['rehypePlugins']>) {
  let entry = runPlugins.get(animatePlugin);
  if (!entry) {
    const state = createGlyphRuns();
    const plugin = () => (tree: Parameters<typeof applyGlyphRuns>[1]) => applyGlyphRuns(state, tree, performance.now(), RUN_FADE_MS);
    // streamdown (2.6) caches its unified processors by the plugins' function names, so one name for every block ran the
    // first block's run state everywhere: a seeded resume never reached the state that drew, and a new block faded only
    // past the character count of another one
    Object.defineProperty(plugin, 'name', { value: `acpGlyphRuns${++glyphRunPluginSeq}` });
    entry = { state, list: rehypePlugins.map(p => (p === animatePlugin.rehypePlugin ? plugin : p)) };
    runPlugins.set(animatePlugin, entry);
  }
  return entry;
}

function SettlingBlock(props: BlockProps) {
  const live = useContext(LiveBlocks);
  const { animatePlugin, rehypePlugins } = props;
  if (!live || !animatePlugin || !rehypePlugins) return <Block {...props} />;
  if (props.index >= live.count - ANIMATED_TAIL_BLOCKS) {
    if (live.runs) {
      const runs = glyphRunPlugins(animatePlugin, rehypePlugins);
      if (live.resume) seedGlyphRuns(runs.state);
      return <Block {...props} animatePlugin={null} rehypePlugins={runs.list} />;
    }
    // The plugin counts rendered characters, which never exceed the markdown source, so every glyph of this
    // render counts as already shown; Block's commit then records the real count for the next chunk
    if (live.resume) animatePlugin.setPrevContentLength(props.content.length);
    return <Block {...props} />;
  }
  let plain = settledPlugins.get(rehypePlugins);
  if (!plain) {
    plain = rehypePlugins.filter(plugin => plugin !== animatePlugin.rehypePlugin);
    settledPlugins.set(rehypePlugins, plain);
  }
  return <Block {...props} animatePlugin={null} rehypePlugins={plain} />;
}

// streamdown routes fenced code through `code` and inline through `inlineCode`, telling them apart by a `data-block`
// marker that its default `pre` cloneElements onto the code child — so the `pre` override below must replicate that
// marker, and the `code` override must handle mermaid itself (MermaidBlock), since the plugin's renderer only runs inside the default component
const COMPONENTS: Components = {
  pre: ({ children }) => (isValidElement(children) ? cloneElement(children as ReactElement<Record<string, unknown>>, { 'data-block': 'true' }) : children),
  code: ({ className, children }) => {
    const lang = /language-(\w+)/.exec(className ?? '')?.[1];
    const code = textOf(children).replace(/\n$/, '');
    return lang === 'mermaid' ? <MermaidBlock chart={code} /> : <CodeBlock code={code} lang={lang} />;
  },
  inlineCode: InlineFileCode,
  a: Link,
  img: MarkdownImage,
  table: ({ children }) => (
    <div className="acp-table scroll-thin overflow-x-auto rounded-lg border border-conversation-line">
      <table>{children}</table>
    </div>
  ),
  // A task notification the model wrote into its reply (`taskNotes.ts`)
  [TASK_NOTE_TAG]: TaskNote,
} as Components;

// One quiet row: who wrote it, the summary it claims, and the task id; the tooltip says the command row holds the real state
function TaskNote(props: Record<string, unknown>) {
  const field = (name: string) => (typeof props[name] === 'string' ? props[name] as string : '');
  const claim = field('data-summary') || field('data-status');
  const task = field('data-task');
  return (
    <Row data-task-note lead={<Bell className="size-icon" strokeWidth={1.5} />} title={t('prose.taskNote.hint')}
      trailing={task ? <span className="font-mono text-mono">{task}</span> : undefined}>
      <RowLabel>{t('prose.taskNote')}</RowLabel>
      {claim && <RowTarget>{claim}</RowTarget>}
    </Row>
  );
}

// The block's host-read local images; COMPONENTS is module-level, so the per-block list travels by context
const MarkdownImages = createContext<ImageRef[] | undefined>(undefined);

// A local image source resolves to the blob the host read; until then (or when the file could not be read) the card names the source.
// Web images keep a plain <img> within the agent image height cap
function MarkdownImage({ src, alt }: ComponentProps<'img'> & ExtraProps) {
  const images = useContext(MarkdownImages);
  const written = typeof src === 'string' ? decodeLocalImageSrc(src) : undefined;
  if (written !== undefined) {
    const hit = images?.find(i => i.uri !== undefined && sameImageSource(i.uri, written));
    return <AgentImage image={hit ?? { mimeType: 'image/png', uri: written }} />;
  }
  return typeof src === 'string' ? <img src={src} alt={alt ?? ''} className="max-h-agent-image max-w-full rounded-md" /> : null;
}

// hast children → plain text (the fenced-code mapping gets elements, not a string)
function textOf(node: ReactNode): string {
  if (node == null || typeof node === 'boolean') return '';
  if (typeof node === 'string' || typeof node === 'number') return String(node);
  if (Array.isArray(node)) return node.map(textOf).join('');
  if (isValidElement(node)) return textOf((node.props as { children?: ReactNode }).children);
  return '';
}

let mmdSeq = 0;

// Mermaid diagram: rendered off-DOM via the plugin's shared instance; while streaming (or on bad syntax) the source shows as a code block
function MermaidBlock({ chart }: { chart: string }) {
  const theme = useTheme();
  const [svg, setSvg] = useState<string>();
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let dead = false;
    setFailed(false);
    // Debounced: the chart text changes with every streamed chunk, and partial syntax usually fails to parse
    const timer = setTimeout(() => {
      const id = `acp-mmd-${++mmdSeq}`;
      mermaidDiagram
        .getMermaid({ startOnLoad: false, theme: theme === 'dark' ? 'dark' : 'default' })
        .render(id, chart)
        .then(({ svg }) => { if (!dead) setSvg(svg); })
        .catch(() => {
          // mermaid drops its error graphic into the document under the render id
          document.getElementById(id)?.remove();
          document.getElementById(`d${id}`)?.remove();
          if (!dead) { setSvg(undefined); setFailed(true); }
        });
    }, 200);
    return () => { dead = true; clearTimeout(timer); };
  }, [chart, theme]);
  if (failed || !svg) return failed ? <CodeBlock code={chart} /> : <div className="min-h-10 animate-[acp-pulse_1.5s_ease_infinite] rounded-md bg-code" />;
  return <div className="flex justify-center [&_svg]:h-auto [&_svg]:max-w-full" dangerouslySetInnerHTML={{ __html: svg }} />;
}
