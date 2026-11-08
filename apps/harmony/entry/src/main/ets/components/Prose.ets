import { Metrics, WebMetrics, WebType } from '../design/Tokens';
import { MdBlock, MdInline, parseMarkdown, plainInline, repairStream } from '../logic/Markdown';
import { codeLanguage, CodeToken, highlightCached } from '../logic/Syntax';
import { tp } from '../logic/Folding';
import { t } from '../store/L10n';
import { copyText, openLink } from './Copy';
import { Lucide } from './Glyphs';

// An agent's Markdown (chat/Prose.tsx + styles/prose.css, apps/ios Prose/Prose.swift, apps/android ui/Prose.kt): blocks
// stack with a --gap; headings, nested lists with GFM task boxes, quotes on a rail, tables in a ringed frame, fenced
// code on the code background with a header naming the language and a copy button, rules, images; inline code chips,
// strong / emphasis / strike and links that open in the browser. `thought` is the quieter reasoning tone. Display math
// shows its TeX source and a mermaid fence stays a code block (the iOS app typesets both natively); a streamed text has
// its unfinished tail repaired first and keeps the blocks before the tail as they were.

/** A flattened inline run: one Span */
export class SpanRun {
  text: string = '';
  code: boolean = false;
  strong: boolean = false;
  em: boolean = false;
  strike: boolean = false;
  url: string = '';
}

function flatten(inlines: MdInline[], into: SpanRun[], strong: boolean, em: boolean, strike: boolean, url: string): void {
  for (const i of inlines) {
    if (i.kind === 'text' || i.kind === 'code' || i.kind === 'br' || i.kind === 'image') {
      const run = new SpanRun();
      // The chip's padding: a narrow space inside its fill on both sides (`.1em .4em`)
      run.text = i.kind === 'br' ? '\n' : i.kind === 'code' ? ` ${i.text} ` : i.kind === 'image' ? i.text : i.text;
      run.code = i.kind === 'code';
      run.strong = strong;
      run.em = em;
      run.strike = strike;
      run.url = url;
      into.push(run);
    } else {
      flatten(i.children, into, strong || i.kind === 'strong', em || i.kind === 'em', strike || i.kind === 'strike',
        i.kind === 'link' ? i.url : url);
    }
  }
}

export function spanRuns(inlines: MdInline[]): SpanRun[] {
  const out: SpanRun[] = [];
  flatten(inlines, out, false, false, false, '');
  return out;
}

/** A syntax role's colour (light-plus / dark-plus through the syn_* resources); '' is the card's ink */
export function synColor(role: string): ResourceColor {
  switch (role) {
    case 'keyword':
      return $r('app.color.syn_keyword');
    case 'control':
      return $r('app.color.syn_control');
    case 'string':
      return $r('app.color.syn_string');
    case 'number':
      return $r('app.color.syn_number');
    case 'comment':
      return $r('app.color.syn_comment');
    case 'function':
      return $r('app.color.syn_function');
    case 'type':
      return $r('app.color.syn_type');
    case 'variable':
      return $r('app.color.syn_variable');
    case 'tag':
      return $r('app.color.syn_tag');
    case 'punct':
      return $r('app.color.syn_punct');
    default:
      return $r('app.color.fg1');
  }
}

/** One coloured run of a code card, newlines included */
export class CodeRun {
  text: string = '';
  role: string = '';
}

/** The code's runs in order, each line's tokens then its newline; plain text is one run */
export function codeRuns(text: string, language: string): CodeRun[] {
  const lines: CodeToken[][] | undefined = highlightCached(text, language);
  const out: CodeRun[] = [];
  if (lines === undefined) {
    const run = new CodeRun();
    run.text = text;
    out.push(run);
    return out;
  }
  lines.forEach((line: CodeToken[], i: number) => {
    for (const token of line) {
      const run = new CodeRun();
      run.text = token.text;
      run.role = token.role;
      out.push(run);
    }
    if (i < lines.length - 1) {
      const nl = new CodeRun();
      nl.text = '\n';
      out.push(nl);
    }
  });
  return out;
}

/** A paragraph's inlines cut at its images: each image alone, the text between as runs, blank runs dropped */
function imageRuns(inlines: MdInline[]): MdInline[][] {
  const out: MdInline[][] = [];
  let run: MdInline[] = [];
  const flush = () => {
    if (plainInline(run).trim().length > 0) {
      out.push(run);
    }
    run = [];
  };
  for (const i of inlines) {
    if (i.kind === 'image') {
      flush();
      out.push([i]);
    } else {
      run.push(i);
    }
  }
  flush();
  return out;
}

/** A block's identity in its stack: its place and kind, plus enough of its content that a changed block draws again */
function blockKey(b: MdBlock, i: number): string {
  let size = b.text.length + b.items.length * 7 + b.rows.length * 13 + b.blocks.length * 17 + b.checks.join('').length;
  size += plainInline(b.inlines).length;
  for (const item of b.items) {
    for (const inner of item) {
      size += blockKey(inner, 0).length + plainInline(inner.inlines).length + inner.text.length;
    }
  }
  for (const inner of b.blocks) {
    size += plainInline(inner.inlines).length + inner.text.length + inner.items.length;
  }
  return `${i}:${b.kind}:${b.level}:${size}`;
}

/** Inline content in one Text: code chips, strong / emphasis / strike, links in the accent that open on a tap */
@ComponentV2
export struct InlineText {
  @Param inlines: MdInline[] = [];
  @Param fontSize: number = WebType.text1;
  @Param lineHeight: number = WebType.text1Line;
  @Param weight: FontWeight = FontWeight.Normal;
  @Param ink: ResourceColor = $r('app.color.fg1');
  @Param strongInk: ResourceColor = $r('app.color.fg_strong');
  @Param oneLine: boolean = false;

  build() {
    Text() {
      ForEach(spanRuns(this.inlines), (s: SpanRun) => {
        if (s.code) {
          Span(s.text)
            .fontFamily('monospace')
            .fontSize(WebType.mono)
            .fontColor(s.url.length > 0 ? $r('app.color.command') : $r('app.color.fg1'))
            .textBackgroundStyle({ color: $r('app.color.hover'), radius: Metrics.rSm })
            .onClick(() => {
              if (s.url.length > 0) {
                openLink(this.getUIContext(), s.url);
              }
            })
        } else {
          Span(s.text)
            .fontWeight(s.strong ? FontWeight.Medium : this.weight)
            .fontStyle(s.em ? FontStyle.Italic : FontStyle.Normal)
            .decoration({ type: s.strike ? TextDecorationType.LineThrough : TextDecorationType.None })
            .fontColor(s.url.length > 0 ? $r('app.color.command') : (s.strong ? this.strongInk : this.ink))
            .onClick(() => {
              if (s.url.length > 0) {
                openLink(this.getUIContext(), s.url);
              }
            })
        }
      }, (s: SpanRun, i: number) => `${i}:${s.text.length}:${s.code}:${s.strong}:${s.em}:${s.url.length}`)
    }
    .fontSize(this.fontSize)
    .lineHeight(this.lineHeight)
    .fontWeight(this.weight)
    .fontColor(this.ink)
    .maxLines(this.oneLine ? 1 : WebMetrics.allLines)
    .copyOption(CopyOptions.LocalDevice)
    .width(this.oneLine ? undefined : '100%')
  }
}

/**
 * CodeSurface: one fence on the code background inside the conversation ring, its header naming the language with a
 * copy button, the code scrolling sideways and coloured when the language is known
 */
@ComponentV2
export struct CodeSurface {
  @Param text: string = '';
  @Param lang: string = '';

  build() {
    Column() {
      Row() {
        Text(this.lang.length > 0 ? this.lang : 'text')
          .fontSize(WebType.text3)
          .lineHeight(WebType.text3Line)
          .fontColor($r('app.color.fg3'))
          .maxLines(1)
          .layoutWeight(1)
        Stack() {
          Lucide({ name: 'copy', tint: $r('app.color.fg3'), extent: WebMetrics.icon, weight: WebMetrics.strokeIcon })
        }
        .width(WebMetrics.ctlSm)
        .height(WebMetrics.ctlSm)
        .borderRadius(Metrics.rSm)
        .stateStyles({
          pressed: {
            .backgroundColor($r('app.color.hover'))
          },
          normal: {
            .backgroundColor(Color.Transparent)
          }
        })
        .accessibilityText(t('common.copy'))
        .onClick(() => copyText(this.getUIContext(), this.text))
      }
      .height(WebMetrics.codeHead)
      .padding({ left: Metrics.pad, right: Metrics.s1 })
      .width('100%')

      Scroll() {
        Text() {
          ForEach(codeRuns(this.text, codeLanguage(this.lang)), (r: CodeRun) => {
            Span(r.text).fontColor(synColor(r.role))
          }, (r: CodeRun, i: number) => `${i}:${r.role}:${r.text.length}`)
        }
        .fontFamily('monospace')
        .fontSize(WebType.mono)
        .lineHeight(WebType.monoLine)
        .fontColor($r('app.color.fg1'))
        .copyOption(CopyOptions.LocalDevice)
        .padding({ left: Metrics.pad, right: Metrics.pad, bottom: Metrics.pad - WebMetrics.s0_5 })
      }
      .scrollable(ScrollDirection.Horizontal)
      .scrollBar(BarState.Off)
      .edgeEffect(EdgeEffect.None)
      .align(Alignment.TopStart)
      .width('100%')
    }
    .backgroundColor($r('app.color.bg_code'))
    .borderRadius(Metrics.rLg)
    .border({ width: Metrics.hairline, color: $r('app.color.conversation_line') })
    .clip(true)
    .width('100%')
  }
}

/**
 * A table (prose.css): the header on --hover, text-2 cells padded gap × pad on one line each, a hairline under every row
 * but the last, the frame rounded and ringed, at least the column's width and scrolling sideways when wider. Laid out by
 * column, each cell one line tall, so the rows line up across the columns
 */
@ComponentV2
export struct ProseTable {
  @Param block: MdBlock = new MdBlock();
  @Param ink: ResourceColor = $r('app.color.fg1');
  @Param strongInk: ResourceColor = $r('app.color.fg_strong');

  columnAlign(c: number): HorizontalAlign {
    const a = c < this.block.align.length ? this.block.align[c] : 'none';
    return a === 'center' ? HorizontalAlign.Center : a === 'right' ? HorizontalAlign.End : HorizontalAlign.Start;
  }

  cell(r: number, c: number): MdInline[] {
    if (r === 0) {
      return c < this.block.header.length ? this.block.header[c] : [];
    }
    const row = this.block.rows[r - 1];
    return c < row.length ? row[c] : [];
  }

  /**
   * A column's natural width: its widest cell on one line plus the cell padding. Columns get explicit widths (growing to
   * fill the frame), since a percentage width inside the sideways scroll resolves against the viewport
   */
  columnWidth(c: number): number {
    const ui = this.getUIContext();
    let widest = 0;
    for (let r = 0; r <= this.block.rows.length; r++) {
      let px = 0;
      for (const run of spanRuns(this.cell(r, c))) {
        px += ui.getMeasureUtils().measureText({
          textContent: run.text, fontSize: run.code ? WebType.mono : WebType.text2,
          fontFamily: run.code ? 'monospace' : undefined,
          fontWeight: r === 0 || run.strong ? FontWeight.Medium : FontWeight.Normal
        });
      }
      widest = Math.max(widest, ui.px2vp(px));
    }
    return Math.ceil(widest) + Metrics.pad * 2 + Metrics.hairline;
  }

  build() {
    Scroll() {
      Row() {
        ForEach(this.block.header, (h: MdInline[], c: number) => {
          Column() {
            ForEach([0].concat(this.block.rows.map((row: MdInline[][], r: number) => r + 1)), (r: number) => {
              Column() {
                InlineText({
                  inlines: this.cell(r, c), fontSize: WebType.text2, lineHeight: WebType.text2Line,
                  weight: r === 0 ? FontWeight.Medium : FontWeight.Normal, ink: r === 0 ? $r('app.color.fg2') : this.ink,
                  strongInk: this.strongInk, oneLine: true
                })
              }
              .alignItems(this.columnAlign(c))
              .justifyContent(FlexAlign.Center)
              .padding({ left: Metrics.pad, right: Metrics.pad })
              .height(WebType.text2Line + Metrics.gap * 2)
              .width('100%')
              .backgroundColor(r === 0 ? $r('app.color.hover') : Color.Transparent)
              .border({
                width: { bottom: r < this.block.rows.length ? Metrics.hairline : 0 },
                color: { bottom: $r('app.color.line') }
              })
            }, (r: number) => `${r}`)
          }
          .width(this.columnWidth(c))
          .flexGrow(1)
        }, (h: MdInline[], c: number) => `${c}:${plainInline(h)}`)
      }
      .constraintSize({ minWidth: '100%' })
    }
    .scrollable(ScrollDirection.Horizontal)
    .scrollBar(BarState.Off)
    .edgeEffect(EdgeEffect.None)
    .align(Alignment.TopStart)
    .borderRadius(Metrics.rLg)
    .border({ width: Metrics.hairline, color: $r('app.color.conversation_line') })
    .clip(true)
    .width('100%')
  }
}

/**
 * A Markdown image as an agent image card (MediaViews.swift ImageParagraph): a web address loads in place; anything
 * else (a path on the computer, a blob the engine serves) shows its alt text on a placeholder until the media layer
 * fetches it
 */
@ComponentV2
export struct ProseImage {
  @Param url: string = '';
  @Param alt: string = '';
  @Local failed: boolean = false;

  build() {
    if (/^https?:/i.test(this.url) && !this.failed) {
      Image(this.url)
        .objectFit(ImageFit.Contain)
        .constraintSize({ maxHeight: WebMetrics.imageMax, maxWidth: '100%' })
        .borderRadius(Metrics.rLg)
        .onError(() => {
          this.failed = true;
        })
    } else {
      Row({ space: Metrics.s2 }) {
        Lucide({ name: 'image', tint: $r('app.color.fg3'), extent: WebMetrics.icon, weight: WebMetrics.strokeIcon })
        Text(this.alt.length > 0 ? this.alt : this.url)
          .fontSize(WebType.text3)
          .lineHeight(WebType.text3Line)
          .fontColor($r('app.color.fg2'))
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
          .flexShrink(1)
      }
      .padding({ left: Metrics.gap, right: Metrics.gap })
      .height(WebMetrics.ctl)
      .backgroundColor($r('app.color.chip'))
      .borderRadius(Metrics.rLg)
    }
  }
}

/** The tone's inks: a reply in fg-1 with fg-strong emphasis; a thought (and a quote) in fg-2 with fg-1 emphasis */
function bodyInk(tone: string): ResourceColor {
  return tone === 'reply' ? $r('app.color.fg1') : $r('app.color.fg2');
}

function strongInk(tone: string): ResourceColor {
  return tone === 'reply' ? $r('app.color.fg_strong') : $r('app.color.fg1');
}

function headingSize(tone: string, level: number): number {
  if (tone !== 'reply') {
    return WebType.text1;
  }
  return level === 1 ? WebType.h1 : level === 2 ? WebType.h2 : level === 3 ? WebType.h3 : WebType.text1;
}

/** A stack of blocks; list items and quotes stack their own blocks in a nested one */
@ComponentV2
export struct ProseStack {
  @Param blocks: MdBlock[] = [];
  /** reply, thought, or quote (a thought's inks at the reply's scale) */
  @Param tone: string = 'reply';
  /** The space between blocks: a gap at the top level, 2 vp inside a list item */
  @Param space: number = Metrics.gap;
  @Local shown: number = WebMetrics.prosePage;

  @Builder
  paragraph(b: MdBlock) {
    if (!b.inlines.some((i: MdInline) => i.kind === 'image')) {
      InlineText({ inlines: b.inlines, ink: bodyInk(this.tone), strongInk: strongInk(this.tone) })
    } else {
      Column({ space: Metrics.gap }) {
        ForEach(imageRuns(b.inlines), (run: MdInline[]) => {
          if (run.length === 1 && run[0].kind === 'image') {
            ProseImage({ url: run[0].url, alt: run[0].text })
          } else {
            InlineText({ inlines: run, ink: bodyInk(this.tone), strongInk: strongInk(this.tone) })
          }
        }, (run: MdInline[], i: number) => `${i}:${plainInline(run).length}`)
      }
      .alignItems(HorizontalAlign.Start)
      .width('100%')
    }
  }

  @Builder
  list(b: MdBlock) {
    Column({ space: WebMetrics.s0_5 }) {
      ForEach(b.items.slice(0, this.shown), (item: MdBlock[], i: number) => {
        Row() {
          // The marker column: "1." or a disc right-aligned, a task box in place of the disc
          Row() {
            if (i < b.checks.length && b.checks[i] >= 0) {
              Lucide({
                name: b.checks[i] === 1 ? 'squareCheck' : 'square', tint: $r('app.color.fg3'), extent: WebMetrics.icon,
                weight: WebMetrics.strokeIcon
              })
            } else if (b.ordered) {
              Text(`${b.start + i}.`)
                .fontSize(WebType.text1)
                .lineHeight(WebType.text1Line)
                .fontColor($r('app.color.fg3'))
                .maxLines(1)
            } else {
              Circle()
                .width(WebMetrics.disc)
                .height(WebMetrics.disc)
                .fill($r('app.color.fg3'))
            }
          }
          .justifyContent(FlexAlign.End)
          .padding({ right: WebMetrics.markerSpace })
          .height(WebType.text1Line)
          .constraintSize({ minWidth: WebMetrics.listIndent })
          ProseStack({ blocks: item, tone: this.tone, space: WebMetrics.s0_5 })
            .layoutWeight(1)
        }
        .alignItems(VerticalAlign.Top)
        .padding({ top: Metrics.s1, bottom: Metrics.s1 })
        .width('100%')
      }, (item: MdBlock[], i: number) => `${i}:${b.checks[i] ?? -1}:${item.map((x: MdBlock, k: number) => blockKey(x, k)).join(',')}`)
      if (b.items.length > this.shown) {
        Text(tp('session.showMore', 'n', `${b.items.length - this.shown}`))
          .fontSize(WebType.text2)
          .fontColor($r('app.color.fg3'))
          .padding({ left: WebMetrics.listIndent })
          .onClick(() => {
            this.shown += WebMetrics.prosePage;
          })
      }
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
  }

  build() {
    Column({ space: this.space }) {
      ForEach(this.blocks.slice(0, this.shown), (b: MdBlock, i: number) => {
        if (b.kind === 'p') {
          this.paragraph(b)
        } else if (b.kind === 'heading') {
          InlineText({
            inlines: b.inlines, fontSize: headingSize(this.tone, b.level), lineHeight: WebType.text1Line,
            weight: FontWeight.Medium, ink: this.tone === 'reply' ? $r('app.color.fg_strong') : strongInk(this.tone),
            strongInk: this.tone === 'reply' ? $r('app.color.fg_strong') : strongInk(this.tone)
          })
            // :not(:first-child) gets margin-top: var(--gap) on top of the block gap
            .margin({ top: i === 0 ? 0 : Metrics.gap })
        } else if (b.kind === 'list') {
          this.list(b)
        } else if (b.kind === 'quote') {
          // The rail is the quote's own left edge, so it runs exactly the quote's height
          Column() {
            ProseStack({ blocks: b.blocks, tone: 'quote' })
          }
          .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.gap, bottom: Metrics.gap })
          .border({ width: { left: WebMetrics.quoteRail }, color: { left: $r('app.color.line_strong') } })
          .width('100%')
        } else if (b.kind === 'code') {
          CodeSurface({ text: b.text, lang: b.lang })
        } else if (b.kind === 'table') {
          ProseTable({ block: b, ink: bodyInk(this.tone), strongInk: strongInk(this.tone) })
        } else if (b.kind === 'rule') {
          Divider()
            .strokeWidth(Metrics.hairline)
            .color($r('app.color.conversation_line'))
            .margin({ top: Metrics.gap, bottom: Metrics.gap })
        } else if (b.kind === 'math') {
          // `$$` display math as its TeX source, centred like the typeset block would be
          Text(b.text)
            .fontFamily('monospace')
            .fontSize(WebType.mono)
            .lineHeight(WebType.monoLine)
            .fontColor(bodyInk(this.tone))
            .textAlign(TextAlign.Center)
            .copyOption(CopyOptions.LocalDevice)
            .width('100%')
        }
      }, (b: MdBlock, i: number) => blockKey(b, i))
      if (this.blocks.length > this.shown) {
        Text(tp('session.showMore', 'n', `${this.blocks.length - this.shown}`))
          .fontSize(WebType.text2)
          .fontColor($r('app.color.fg3'))
          .onClick(() => {
            this.shown += WebMetrics.prosePage;
          })
      }
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
  }
}

/**
 * A Markdown document in the transcript's prose. `streaming`: the text is still arriving, so its unfinished tail is
 * repaired first (streamdown's streaming mode); blocks keep their keys while their text stands, so only the growing tail
 * lays out again
 */
@ComponentV2
export struct Prose {
  @Param markdown: string = '';
  @Param tone: string = 'reply';
  @Param streaming: boolean = false;

  @Computed
  get blocks(): MdBlock[] {
    const text = this.streaming ? repairStream(this.markdown) : this.markdown;
    return parseMarkdown(text.replace(/^\n+/, '').replace(/\n+$/, ''));
  }

  build() {
    ProseStack({ blocks: this.blocks, tone: this.tone })
  }
}
