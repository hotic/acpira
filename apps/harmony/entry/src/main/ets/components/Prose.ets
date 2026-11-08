import { Metrics, Type } from '../design/Tokens';

// An agent's Markdown (chat/Prose.tsx, apps/ios Prose): paragraphs, headings, lists, quotes and fenced code, with
// inline code, strong text and links inside them. A subset of the iOS renderer: tables show as code, math and diagrams
// as their source.

class MdBlock {
  kind: string = 'p';
  text: string = '';
  level: number = 0;
  marker: string = '';
}

class MdSpan {
  text: string = '';
  code: boolean = false;
  strong: boolean = false;
  link: boolean = false;
}

function parseBlocks(markdown: string): MdBlock[] {
  const out: MdBlock[] = [];
  const lines = markdown.replace(/\r\n/g, '\n').split('\n');
  let para: string[] = [];
  const flush = () => {
    if (para.length > 0) {
      const b = new MdBlock();
      b.text = para.join(' ');
      out.push(b);
      para = [];
    }
  };
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    const fence = line.match(/^\s*(```|~~~)/);
    if (fence !== null) {
      flush();
      const code: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith(fence[1])) {
        code.push(lines[i]);
        i++;
      }
      const b = new MdBlock();
      b.kind = 'code';
      b.text = code.join('\n');
      out.push(b);
      i++;
      continue;
    }
    const heading = line.match(/^(#{1,6})\s+(.*)$/);
    const item = line.match(/^\s*([-*+]|\d+[.)])\s+(.*)$/);
    const quote = line.match(/^\s*>\s?(.*)$/);
    if (line.trim().length === 0) {
      flush();
    } else if (heading !== null) {
      flush();
      const b = new MdBlock();
      b.kind = 'h';
      b.level = heading[1].length;
      b.text = heading[2];
      out.push(b);
    } else if (item !== null) {
      flush();
      const b = new MdBlock();
      b.kind = 'li';
      b.marker = /\d/.test(item[1]) ? item[1] : '•';
      b.text = item[2];
      out.push(b);
    } else if (quote !== null) {
      flush();
      const b = new MdBlock();
      b.kind = 'quote';
      b.text = quote[1];
      out.push(b);
    } else if (/^\s*(---|\*\*\*|___)\s*$/.test(line)) {
      flush();
      const b = new MdBlock();
      b.kind = 'rule';
      out.push(b);
    } else {
      para.push(line.trim());
    }
    i++;
  }
  flush();
  return out;
}

function parseSpans(text: string): MdSpan[] {
  const out: MdSpan[] = [];
  const pattern = /(`[^`]+`)|(\*\*[^*]+\*\*)|(\[[^\]]+\]\([^)]+\))/g;
  let at = 0;
  let m: RegExpExecArray | null = pattern.exec(text);
  while (m !== null) {
    if (m.index > at) {
      const plain = new MdSpan();
      plain.text = text.slice(at, m.index);
      out.push(plain);
    }
    const s = new MdSpan();
    const token = m[0];
    if (token.startsWith('`')) {
      s.code = true;
      s.text = token.slice(1, -1);
    } else if (token.startsWith('**')) {
      s.strong = true;
      s.text = token.slice(2, -2);
    } else {
      s.link = true;
      s.text = token.slice(1, token.indexOf(']'));
    }
    out.push(s);
    at = m.index + token.length;
    m = pattern.exec(text);
  }
  if (at < text.length) {
    const rest = new MdSpan();
    rest.text = text.slice(at);
    out.push(rest);
  }
  return out;
}

@ComponentV2
export struct Prose {
  @Param markdown: string = '';
  @Param ink: ResourceColor = $r('app.color.fg1');
  @Local blocks: MdBlock[] = [];

  @Monitor('markdown')
  onMarkdown(): void {
    this.blocks = parseBlocks(this.markdown);
  }

  aboutToAppear(): void {
    this.blocks = parseBlocks(this.markdown);
  }

  @Builder
  inline(text: string, size: number, line: number, weight: FontWeight, ink: ResourceColor) {
    Text() {
      ForEach(parseSpans(text), (s: MdSpan) => {
        if (s.code) {
          Span(s.text)
            .fontFamily('monospace')
            .fontSize(Type.mono)
            .fontColor($r('app.color.fg1'))
            .textBackgroundStyle({ color: $r('app.color.chip'), radius: Metrics.rSm })
        } else {
          Span(s.text)
            .fontWeight(s.strong ? FontWeight.Medium : weight)
            .fontColor(s.link ? $r('app.color.command') : (s.strong ? $r('app.color.fg_strong') : ink))
        }
      }, (s: MdSpan, i: number) => `${i}`)
    }
    .fontSize(size)
    .lineHeight(line)
    .fontWeight(weight)
    .fontColor(ink)
    .copyOption(CopyOptions.LocalDevice)
    .width('100%')
  }

  build() {
    Column({ space: Metrics.gap }) {
      ForEach(this.blocks, (b: MdBlock) => {
        if (b.kind === 'code') {
          Scroll() {
            Text(b.text)
              .fontFamily('monospace')
              .fontSize(Type.mono)
              .lineHeight(Type.monoLine)
              .fontColor($r('app.color.fg1'))
              .copyOption(CopyOptions.LocalDevice)
              .padding(Metrics.gap)
          }
          .scrollable(ScrollDirection.Horizontal)
          .scrollBar(BarState.Off)
          .width('100%')
          .align(Alignment.Start)
          .backgroundColor($r('app.color.bg_code'))
          .borderRadius(Metrics.rMd)
        } else if (b.kind === 'h') {
          this.inline(b.text, b.level <= 2 ? Type.barTitle : Type.body, Type.bodyLine, FontWeight.Medium,
            $r('app.color.fg_strong'))
        } else if (b.kind === 'li') {
          Row() {
            Text(b.marker)
              .fontSize(Type.body)
              .lineHeight(Type.bodyLine)
              .fontColor($r('app.color.fg2'))
              .width(Metrics.lead)
            this.inline(b.text, Type.body, Type.bodyLine, FontWeight.Normal, this.ink)
          }
          .alignItems(VerticalAlign.Top)
          .width('100%')
        } else if (b.kind === 'quote') {
          Row({ space: Metrics.gap }) {
            Divider()
              .vertical(true)
              .strokeWidth(Metrics.s1)
              .color($r('app.color.line_strong'))
              .height(Type.bodyLine)
            this.inline(b.text, Type.body, Type.bodyLine, FontWeight.Normal, $r('app.color.fg2'))
          }
          .width('100%')
        } else if (b.kind === 'rule') {
          Divider().strokeWidth(Metrics.hairline).color($r('app.color.line'))
        } else {
          this.inline(b.text, Type.body, Type.bodyLine, FontWeight.Normal, this.ink)
        }
      }, (b: MdBlock, i: number) => `${i}:${b.kind}:${b.text.length}`)
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
  }
}
