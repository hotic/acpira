import { Metrics, WebMetrics, WebType } from '../design/Tokens';
import { Block, DiffLine, ToolContent } from '../wire/Blocks';
import { diffCode, diffCopyText, outputOf, toolVerb } from '../logic/Folding';
import { codeLanguage, CodeToken, highlightCached } from '../logic/Syntax';
import { t } from '../store/L10n';
import { copyText } from './Copy';
import { Lucide } from './Glyphs';
import { CodeRun, synColor } from './Prose';

// The tool cards' output (components/ToolViews.ets; apps/ios Transcript/ToolOutput.swift, apps/android
// ui/ToolOutput.kt): code cards, terminal output and diffs

/** A shell command handed over verbatim: one line on the row, the whole command in the card */
export function shellCommand(b: Block): string {
  const target = (b.target ?? '').trim();
  return b.kind === 'execute' && b.targetMono === true && b.verbKey === undefined && target.length > 0 ? (b.target ?? '') : '';
}

// Code cards

/** What a code card shows (one shape for every card, so the sheet can draw it whole): terminal, diff, wrap, plain */
export class CodeBody {
  kind: string = 'wrap';
  title: string = '';
  copy: string = '';
  copyLabel: string = '';
  command: string = '';
  text: string = '';
  content: ToolContent | undefined = undefined;
}

/** A card's body: cut for the transcript, whole for the sheet */
@Builder
function codeBody(b: CodeBody, whole: boolean) {
  if (b.kind === 'terminal') {
    Column() {
      if (b.command.length > 0) {
        CommandPrompt({ command: b.command, last: b.text.length === 0, whole: whole })
      }
      if (b.text.length > 0) {
        OutputText({ text: b.text, limit: whole ? 0 : PREVIEW_LINES, top: b.command.length > 0 ? Metrics.gapHalf : WebMetrics.commandY })
      }
    }
    .width('100%')
  } else if (b.kind === 'diff') {
    DiffRows({ content: b.content ?? emptyContent('diff'), title: b.title, whole: whole })
  } else {
    Text(b.text.trimEnd())
      .fontFamily('monospace')
      .fontSize(WebType.mono)
      .lineHeight(WebType.monoLine)
      .fontColor($r('app.color.fg2'))
      .copyOption(CopyOptions.LocalDevice)
      .padding({ left: WebMetrics.commandX, right: WebMetrics.commandX, top: WebMetrics.commandY, bottom: WebMetrics.commandY })
      .width('100%')
  }
}

function emptyContent(type: string): ToolContent {
  const c: ToolContent = { type: type };
  return c;
}

/**
 * The `code-output` frame (rounded-md, a hairline, the code background) around a card cut at --code-output-max: the cut
 * does not scroll (long lines still scroll sideways), a fade at the cut shows there is more, and a tap opens the whole
 * card in a sheet with its copy button
 */
@ComponentV2
export struct CodeFrame {
  @Param body: CodeBody = new CodeBody();
  @Local natural: number = 0;
  @Local sheet: boolean = false;

  @Builder
  whole() {
    Column() {
      Row({ space: Metrics.s2 }) {
        Text(this.body.title)
          .fontSize(WebType.text1)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg_strong'))
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
            .ellipsisMode(EllipsisMode.CENTER)
          .layoutWeight(1)
        Stack() {
          Lucide({ name: 'copy', tint: $r('app.color.fg1'), extent: Metrics.iconCtl })
        }
        .width(Metrics.barCtl)
        .height(Metrics.barCtl)
        .accessibilityText(this.body.copyLabel)
        .onClick(() => copyText(this.getUIContext(), this.body.copy))
      }
      .padding({ left: Metrics.pad, right: Metrics.s1 })
      .width('100%')
      Scroll() {
        Column() {
          codeBody(this.body, true)
        }
        .backgroundColor($r('app.color.bg_code'))
        .borderRadius(Metrics.rMd)
        .border({ width: Metrics.hairline, color: $r('app.color.line') })
        .clip(true)
        .margin({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2, bottom: Metrics.pad })
      }
      .scrollBar(BarState.Off)
      .align(Alignment.TopStart)
      .layoutWeight(1)
    }
    .backgroundColor($r('app.color.bg0'))
    .height('100%')
    .width('100%')
  }

  build() {
    Stack({ alignContent: Alignment.Bottom }) {
      Scroll() {
        Column() {
          codeBody(this.body, false)
        }
        .width('100%')
        .onAreaChange((before: Area, after: Area) => {
          this.natural = Number(after.height);
        })
      }
      .enableScrollInteraction(false)
      .scrollBar(BarState.Off)
      .align(Alignment.TopStart)
      .constraintSize({ maxHeight: WebMetrics.codeOutputMax })
      .width('100%')
      if (this.natural > WebMetrics.codeOutputMax) {
        Column()
          .height(WebMetrics.scrollFade)
          .width('100%')
          .linearGradient({ angle: 180, colors: [[Color.Transparent, 0], [$r('app.color.bg_code'), 1]] })
          .hitTestBehavior(HitTestMode.None)
      }
    }
    .backgroundColor($r('app.color.bg_code'))
    .borderRadius(Metrics.rMd)
    .border({ width: Metrics.hairline, color: $r('app.color.line') })
    .clip(true)
    .width('100%')
    .onClick(() => {
      this.sheet = true;
    })
    // A wide window hangs a sheet off its host as a popup by default: centred there, the bottom sheet on a phone as before
    .bindSheet(this.sheet, this.whole(), {
      preferType: SheetType.CENTER, height: SheetSize.LARGE, dragBar: true, showClose: false, onDisappear: () => {
        this.sheet = false;
      }
    })
  }
}

/** The lines the card can show: the cut never scrolls, so a streamed output is laid out only a line past it */
const PREVIEW_LINES: number = Math.ceil(WebMetrics.codeOutputMax / WebType.monoLine) + 1;

/** One run of command output: npm banners muted, a leading ✓ in the success colour */
class OutputRun {
  text: string = '';
  /** '', faint, ok */
  ink: string = '';
}

function outputRuns(text: string, limit: number): OutputRun[] {
  let lines = text.trimEnd().split('\n');
  if (limit > 0 && lines.length > limit) {
    lines = lines.slice(0, limit);
  }
  const out: OutputRun[] = [];
  const push = (s: string, ink: string) => {
    const r = new OutputRun();
    r.text = s;
    r.ink = ink;
    out.push(r);
  };
  lines.forEach((line: string, i: number) => {
    const end = i < lines.length - 1 ? '\n' : '';
    const check = line.indexOf('✓');
    if (line.startsWith('> ') || line.startsWith('Done in ')) {
      push(line + end, 'faint');
    } else if (check >= 0 && line.slice(0, check).trim().length === 0) {
      push(line.slice(0, check), '');
      push('✓', 'ok');
      push(line.slice(check + 1) + end, '');
    } else {
      push(line + end, '');
    }
  });
  return out;
}

function outputInk(ink: string): ResourceColor {
  return ink === 'ok' ? $r('app.color.ok') : ink === 'faint' ? $r('app.color.fg3') : $r('app.color.fg2');
}

/** The output text of a command card: the first lines in the transcript, every line in the sheet */
@ComponentV2
struct OutputText {
  @Param text: string = '';
  @Param limit: number = 0;
  @Param top: number = WebMetrics.commandY;

  build() {
    Scroll() {
      Text() {
        ForEach(outputRuns(this.text, this.limit), (r: OutputRun) => {
          Span(r.text).fontColor(outputInk(r.ink))
        }, (r: OutputRun, i: number) => `${i}:${r.ink}:${r.text.length}`)
      }
      .fontFamily('monospace')
      .fontSize(WebType.mono)
      .lineHeight(WebType.monoLine)
      .fontColor($r('app.color.fg2'))
      .copyOption(CopyOptions.LocalDevice)
      .padding({ left: WebMetrics.commandX, right: WebMetrics.commandX, top: this.top, bottom: WebMetrics.commandY })
    }
    .scrollable(ScrollDirection.Horizontal)
    .scrollBar(BarState.Off)
    .edgeEffect(EdgeEffect.None)
    .align(Alignment.TopStart)
    .width('100%')
  }
}

/**
 * The full command behind a muted `$`, cut to three lines until expanded; wrapped lines hang under the command, not the
 * `$`, and a cut command expands from anywhere on its lines
 */
@ComponentV2
struct CommandPrompt {
  @Param command: string = '';
  @Param last: boolean = true;
  @Param whole: boolean = false;
  @Local expanded: boolean = false;
  /** The command's height laid out whole, against the cut's */
  @Local full: number = 0;

  clipped(): boolean {
    return !this.whole && this.full > WebType.monoLine * WebMetrics.commandPromptLines + Metrics.hairline;
  }

  build() {
    Row() {
      Text('$ ')
        .fontFamily('monospace')
        .fontSize(WebType.mono)
        .lineHeight(WebType.monoLine)
        .fontColor($r('app.color.fg3'))
      Column({ space: WebMetrics.s0_5 }) {
        Stack({ alignContent: Alignment.TopStart }) {
          Text(this.command)
            .fontFamily('monospace')
            .fontSize(WebType.mono)
            .lineHeight(WebType.monoLine)
            .fontColor($r('app.color.fg3'))
            .maxLines(this.expanded || this.whole ? WebMetrics.allLines : WebMetrics.commandPromptLines)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
            .width('100%')
          // Laid out whole but unseen and out of the flow: its height says whether the cut hides anything
          Text(this.command)
            .fontFamily('monospace')
            .fontSize(WebType.mono)
            .lineHeight(WebType.monoLine)
            .visibility(Visibility.Hidden)
            .position({ x: 0, y: 0 })
            .width('100%')
            .onAreaChange((before: Area, after: Area) => {
              this.full = Number(after.height);
            })
        }
        .width('100%')
        if (this.clipped()) {
          Stack() {
            Lucide({ name: 'chevronDown', tint: $r('app.color.fg3'), extent: WebMetrics.chevron, weight: WebMetrics.strokeChevron })
              .rotate({ angle: this.expanded ? 180 : 0 })
              .animation({ duration: WebMetrics.foldMs, curve: Curve.Smooth })
          }
          .padding({ left: Metrics.gapHalf, right: Metrics.gapHalf })
          .backgroundColor($r('app.color.hover'))
          .borderRadius(Metrics.rSm)
        }
      }
      .alignItems(HorizontalAlign.Start)
      .layoutWeight(1)
    }
    .alignItems(VerticalAlign.Top)
    .padding({
      left: WebMetrics.commandX, right: WebMetrics.commandX, top: WebMetrics.commandY,
      bottom: this.last ? WebMetrics.commandY : 0
    })
    .width('100%')
    .onClick(() => {
      if (this.clipped()) {
        this.getUIContext().animateTo({ duration: WebMetrics.foldMs, curve: Curve.Smooth }, () => {
          this.expanded = !this.expanded;
        });
      }
    })
  }
}

/** Command output (Codex-style Shell card): the full command behind a muted `$`, then the output */
@ComponentV2
export struct TerminalOutput {
  @Param block: Block = { type: 'tool_call' };

  body(): CodeBody {
    const b = new CodeBody();
    b.kind = 'terminal';
    b.command = shellCommand(this.block);
    b.text = outputOf(this.block);
    b.title = b.command.length > 0 ? b.command : toolVerb(this.block);
    b.copy = (b.command.length > 0 ? `$ ${b.command}\n` : '') + b.text;
    b.copyLabel = t('code.copyOutput');
    return b;
  }

  build() {
    if (outputOf(this.block).length > 0 || shellCommand(this.block).length > 0) {
      CodeFrame({ body: this.body() })
    }
  }
}

/** Text a non-command tool returned, wrapped (reports and prose, not column-aligned logs) */
@ComponentV2
export struct ToolOutput {
  @Param text: string = '';

  body(): CodeBody {
    const b = new CodeBody();
    b.text = this.text.trimEnd();
    b.title = t('code.copyToolOutput');
    b.copy = b.text;
    b.copyLabel = t('code.copyToolOutput');
    return b;
  }

  build() {
    if (this.text.trim().length > 0) {
      CodeFrame({ body: this.body() })
    }
  }
}

// Diff

/** Enough rows to paint past the transcript's cut, including a leading hunk header (which is not drawn) */
function diffPreview(lines: DiffLine[]): DiffLine[] {
  const shortest = Math.min(WebType.codeOutputLine, WebMetrics.diffHunk + Metrics.gapHalf * 2);
  const limit = Math.ceil(WebMetrics.codeOutputMax / shortest) + 1;
  return lines.slice(0, limit + (lines.length > 0 && lines[0].kind === 'hunk' ? 1 : 0));
}

/** The diff's rows coloured by the file's grammar: one token list per row, undefined for an unknown language */
function diffTokens(lines: DiffLine[], path: string): CodeToken[][] | undefined {
  const ext = path.match(/\.([A-Za-z0-9]+)$/);
  const lang = codeLanguage(ext !== null ? ext[1] : '');
  if (lang.length === 0) {
    return undefined;
  }
  return highlightCached(lines.map((l: DiffLine) => l.kind === 'hunk' ? '' : diffCode(l)).join('\n'), lang);
}

/** A row's code as coloured runs */
function rowRuns(line: DiffLine, tokens: CodeToken[] | undefined): CodeRun[] {
  const code = diffCode(line);
  if (tokens === undefined || code.length === 0) {
    const r = new CodeRun();
    r.text = code.length > 0 ? code : '​';
    return [r];
  }
  return tokens.map((tk: CodeToken) => {
    const r = new CodeRun();
    r.text = tk.text;
    r.role = tk.role;
    return r;
  });
}

/** One diff row: line number, sign and code on the row's add / del tint */
@ComponentV2
struct DiffRow {
  @Param line: DiffLine = { kind: 'ctx', text: ' ' };
  @Param tokens: CodeToken[] | undefined = undefined;

  build() {
    Row() {
      Text(`${(this.line.kind === 'del' ? this.line.oldLine : this.line.newLine) ?? ''}`)
        .fontFamily('monospace')
        .fontSize(WebType.mono)
        .fontColor($r('app.color.fg3'))
        .fontFeature('"tnum" on')
        .textAlign(TextAlign.End)
        .maxLines(1)
        .width(WebMetrics.diffNumber)
      Text(this.line.kind === 'add' ? '+' : this.line.kind === 'del' ? '−' : ' ')
        .fontFamily('monospace')
        .fontSize(WebType.mono)
        .fontColor(this.line.kind === 'add' ? $r('app.color.ok') : this.line.kind === 'del' ? $r('app.color.danger') : $r('app.color.fg3'))
        .textAlign(TextAlign.Center)
        .width(WebMetrics.diffSign)
      Text() {
        ForEach(rowRuns(this.line, this.tokens), (r: CodeRun) => {
          Span(r.text).fontColor(synColor(r.role))
        }, (r: CodeRun, i: number) => `${i}:${r.role}:${r.text}`)
      }
      .fontFamily('monospace')
      .fontSize(WebType.mono)
      .fontColor($r('app.color.fg1'))
      .maxLines(1)
      .padding({ right: Metrics.pad })
    }
    .constraintSize({ minHeight: WebType.codeOutputLine, minWidth: '100%' })
    .backgroundColor(this.line.kind === 'add' ? $r('app.color.diff_add') : this.line.kind === 'del' ? $r('app.color.diff_del') : Color.Transparent)
  }
}

/** The diff's rows, scrolling sideways together; `whole`: every row (the sheet), else the transcript's cut */
@ComponentV2
struct DiffRows {
  @Param content: ToolContent = { type: 'diff' };
  @Param title: string = '';
  @Param whole: boolean = false;

  lines(): DiffLine[] {
    return this.whole ? (this.content.lines ?? []) : diffPreview(this.content.lines ?? []);
  }

  @Computed
  get tokens(): CodeToken[][] | undefined {
    return diffTokens(this.lines(), this.content.source?.path ?? this.title);
  }

  build() {
    Scroll() {
      Column() {
        ForEach(this.lines(), (d: DiffLine, i: number) => {
          if (d.kind === 'hunk') {
            // A leading or trailing hunk header is not drawn
            if (i > 0 && i < this.lines().length - 1) {
              Row() {
                Text('···')
                  .fontSize(WebType.text3)
                  .fontColor($r('app.color.fg3'))
                  .textAlign(TextAlign.Center)
                  .width(WebMetrics.diffNumber + WebMetrics.diffSign)
              }
              .height(WebMetrics.diffHunk)
              .margin({ top: Metrics.gapHalf, bottom: Metrics.gapHalf })
              .border({ width: { top: Metrics.hairline, bottom: Metrics.hairline }, color: $r('app.color.line') })
              .constraintSize({ minWidth: '100%' })
            }
          } else {
            DiffRow({ line: d, tokens: this.tokens !== undefined && i < this.tokens.length ? this.tokens[i] : undefined })
          }
        }, (d: DiffLine, i: number) => `${i}:${d.kind}:${d.text}`)
      }
      .alignItems(HorizontalAlign.Start)
      .constraintSize({ minWidth: '100%' })
      .padding({ top: Metrics.gapHalf, bottom: Metrics.gapHalf })
    }
    .scrollable(ScrollDirection.Horizontal)
    .scrollBar(BarState.Off)
    .edgeEffect(EdgeEffect.None)
    .align(Alignment.TopStart)
    .width('100%')
  }
}

/**
 * DiffBlock: line numbers and signs in a gutter, tinted add / del rows, `···` between hunks. The rows fill at least the
 * card's width, so a tint runs edge to edge however short its line is. Code is coloured by the file's grammar
 */
@ComponentV2
export struct DiffBlockView {
  @Param content: ToolContent = { type: 'diff' };
  @Param title: string = '';

  body(): CodeBody {
    const b = new CodeBody();
    b.kind = 'diff';
    b.content = this.content;
    b.title = this.title;
    b.copy = diffCopyText(this.content);
    b.copyLabel = t('code.copyVisible');
    return b;
  }

  build() {
    CodeFrame({ body: this.body() })
  }
}
