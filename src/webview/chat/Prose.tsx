import { cloneElement, isValidElement, memo, useEffect, useState, type ReactElement, type ReactNode } from 'react';
import { Streamdown, defaultRehypePlugins, type Components } from 'streamdown';
import { createMathPlugin } from '@streamdown/math';
import { mermaid as mermaidDiagram } from '@streamdown/mermaid';
import type { TextBlock } from '@shared/transcript';
import { STREAM_MOTION, useSmoothText, useStreamMotion, type StreamMotion } from './streamMotion';
import { CodeBlock } from './CodeBlock';
import { InlineFileCode, Link } from './Link';
import { rewriteFileHrefs } from './fileLinks';
import { useTheme } from '../look';

// Full Markdown via streamdown: GFM + KaTeX + Mermaid, streaming-aware (remend repairs incomplete syntax mid-stream).
// All typography lives in styles/prose.css under .acp-prose — Tailwind never scans node_modules, so streamdown's own classes don't resolve here.

// singleDollarTextMath stays off: "$5 and $10" in prose must not become math
const PLUGINS = { math: createMathPlugin(), mermaid: mermaidDiagram };
// Module-level: streamdown's top-level memo compares props by reference, so every config object must be stable
const LINK_SAFETY = { enabled: false };
const { raw: rehypeRaw, sanitize: rehypeSanitize, harden: rehypeHarden } = defaultRehypePlugins;
if (!rehypeRaw || !rehypeSanitize || !rehypeHarden) throw new Error('streamdown default rehype plugins missing');
// Rewrite file:// before sanitize/harden; urlTransform runs too late and harden would paint ` [blocked]`.
const REHYPE = [rehypeRaw, rewriteFileHrefs, rehypeSanitize, rehypeHarden];
// `motion` is a stable module-level config (streamdown compares props by reference); the LAB passes alternatives
export const Prose = memo(function Prose({ block, motion = STREAM_MOTION }: { block: TextBlock; motion?: StreamMotion }) {
  const smooth = useSmoothText(block.markdown, !!block.streaming, motion.pace);
  // Still draining counts as streaming: the renderer keeps its streaming mode until the visible text catches up
  const streaming = !!block.streaming || smooth.draining;
  const { animated, animating } = useStreamMotion(streaming, motion);
  // The turn heading already indicates waiting before the first visible words.
  if (!smooth.text.trim()) return null;
  return (
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
      className="acp-prose flex min-w-0 flex-col gap-gap text-1 text-fg-1"
    >
      {smooth.text}
    </Streamdown>
  );
});

// streamdown routes fenced code through `code` and inline through `inlineCode`, telling them apart by a `data-block`
// marker that its default `pre` cloneElements onto the code child — so the `pre` override below must replicate that
// marker, and the `code` override must handle mermaid itself (MermaidBlock), since the plugin's renderer only runs inside the default component
const COMPONENTS: Components = {
  pre: ({ children }) => (isValidElement(children) ? cloneElement(children as ReactElement<Record<string, unknown>>, { 'data-block': 'true' }) : children),
  code: ({ className, children }) => {
    const lang = /language-(\w+)/.exec(className ?? '')?.[1];
    const code = textOf(children).replace(/\n$/, '');
    return lang === 'mermaid' ? <MermaidBlock chart={code} /> : <CodeBlock code={code} />;
  },
  inlineCode: InlineFileCode,
  a: Link,
  table: ({ children }) => (
    <div className="acp-table scroll-thin overflow-x-auto rounded-lg border border-conversation-line">
      <table>{children}</table>
    </div>
  ),
};

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
