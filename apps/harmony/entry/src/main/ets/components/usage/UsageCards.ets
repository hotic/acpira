import { FormMetrics, ListMetrics, Metrics, Type, UsageMetrics } from '../../design/Tokens';
import { t } from '../../store/L10n';
import { figure, spaced } from '../../logic/UsageStats';
import { Lucide } from '../Glyphs';

// Settings → Usage's shared pieces (pages/Usage.ets): the summary tiles, the segmented switch, a card's section head
// and the canvas paths the charts draw with

export class TileItem {
  caption: string = '';
  value: string = '';
  unit: string | undefined = undefined;
}

export class SegChoice {
  value: string = '';
  label: string = '';
}

export class AxisTick {
  x: number = 0;
  label: string = '';
  /** -1 left edge, 0 centred, 1 right edge */
  edge: number = 0;
}

export class TrendCol {
  start: number = 0;
  end: number = 0;
  total: number = 0;
  part: number = 0;
}

export class UsageReading {
  tokens: number = 0;
  date: string | undefined = undefined;
  topModel: string | undefined = undefined;
  activeDays: number | undefined = undefined;
}

export function makeTile(caption: string, value: string, unit: string | undefined): TileItem {
  const item = new TileItem();
  item.caption = caption;
  item.value = value;
  item.unit = unit;
  return item;
}

export function makeSeg(value: string, label: string): SegChoice {
  const c = new SegChoice();
  c.value = value;
  c.label = label;
  return c;
}

export function makeTick(x: number, label: string, edge: number): AxisTick {
  const tk = new AxisTick();
  tk.x = x;
  tk.label = label;
  tk.edge = edge;
  return tk;
}

export function roundRectPath(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number): void {
  const rad = Math.min(r, w / 2, h / 2);
  ctx.beginPath();
  ctx.moveTo(x + rad, y);
  ctx.lineTo(x + w - rad, y);
  ctx.quadraticCurveTo(x + w, y, x + w, y + rad);
  ctx.lineTo(x + w, y + h - rad);
  ctx.quadraticCurveTo(x + w, y + h, x + w - rad, y + h);
  ctx.lineTo(x + rad, y + h);
  ctx.quadraticCurveTo(x, y + h, x, y + h - rad);
  ctx.lineTo(x, y + rad);
  ctx.quadraticCurveTo(x, y, x + rad, y);
  ctx.closePath();
}

export function topRoundBarPath(ctx: CanvasRenderingContext2D, x: number, yTop: number, w: number, h: number, r: number): void {
  const rad = Math.min(r, w / 2, h);
  const yBot = yTop + h;
  ctx.beginPath();
  ctx.moveTo(x, yBot);
  ctx.lineTo(x, yTop + rad);
  ctx.quadraticCurveTo(x, yTop, x + rad, yTop);
  ctx.lineTo(x + w - rad, yTop);
  ctx.quadraticCurveTo(x + w, yTop, x + w, yTop + rad);
  ctx.lineTo(x + w, yBot);
  ctx.closePath();
}

@ComponentV2
export struct UsageSegmented {
  @Param choices: SegChoice[] = [];
  @Param selected: string = '';
  @Event onPick: (value: string) => void = () => {};

  build() {
    Row({ space: ListMetrics.s0_5 }) {
      ForEach(this.choices, (item: SegChoice) => {
        Row() {
          Text(item.label)
            .fontSize(Type.text4)
            .fontWeight(item.value === this.selected ? FontWeight.Medium : FontWeight.Normal)
            .fontColor(item.value === this.selected ? $r('app.color.fg1') : $r('app.color.fg3'))
            .maxLines(1)
        }
        .height(UsageMetrics.segH)
        .padding({ left: UsageMetrics.segPadX, right: UsageMetrics.segPadX })
        .borderRadius(Metrics.rSm)
        .backgroundColor(item.value === this.selected ? $r('app.color.form_row') : Color.Transparent)
        .borderWidth(item.value === this.selected ? Metrics.hairline : 0)
        .borderColor($r('app.color.line'))
        .onClick(() => {
          if (item.value !== this.selected) {
            this.onPick(item.value);
          }
        })
      }, (item: SegChoice) => `${item.value}:${item.value === this.selected}`)
    }
    .padding(UsageMetrics.segPad)
    .borderRadius(Metrics.rMd)
    .backgroundColor($r('app.color.active'))
  }
}

@ComponentV2
export struct UsageSectionHead {
  @Param title: string = '';
  @Param reading: UsageReading | undefined = undefined;
  @Param count: number = -1;
  @BuilderParam trailing: () => void = this.emptyBuilder;

  @Builder
  emptyBuilder() {
  }

  tokenText(r: UsageReading): string {
    if (r.tokens <= 0 && r.date !== undefined) {
      return t('usagePage.noUse');
    }
    const f = figure(r.tokens);
    return spaced(f.value + (f.unit ?? ''));
  }

  build() {
    Row({ space: Metrics.s2 }) {
      Row({ space: Metrics.s2 }) {
        Text(this.title)
          .fontSize(Type.text2)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg1'))
          .maxLines(1)
        if (this.count >= 0) {
          Text(`${this.count}`)
            .fontSize(Type.text4)
            .fontColor($r('app.color.fg3'))
            .maxLines(1)
            .padding({ bottom: Metrics.hairline })
        } else if (this.reading !== undefined) {
          if (this.reading.activeDays !== undefined) {
            Text(t('usagePage.period.days', new Map<string, string>([['n', `${this.reading.activeDays}`]])))
              .fontSize(Type.text4)
              .fontColor($r('app.color.fg3'))
              .maxLines(1)
              .padding({ bottom: Metrics.hairline })
          } else {
            Text(this.tokenText(this.reading))
              .fontSize(Type.text4)
              .fontColor(this.reading.date !== undefined ? $r('app.color.fg2') : $r('app.color.fg3'))
              .maxLines(1)
              .padding({ bottom: Metrics.hairline })
            if (this.reading.topModel !== undefined) {
              Row({ space: Metrics.s1 }) {
                Lucide({ name: 'crown', tint: $r('app.color.fg3'), extent: UsageMetrics.crown })
                Text(this.reading.topModel)
                  .fontSize(Type.text4)
                  .fontColor($r('app.color.fg3'))
                  .maxLines(1)
                  .textOverflow({ overflow: TextOverflow.Ellipsis })
                  .constraintSize({ maxWidth: UsageMetrics.nameWide })
              }
              .flexShrink(1)
              .padding({ bottom: Metrics.hairline })
            }
          }
        }
        Blank()
        if (this.reading?.date !== undefined) {
          Text(this.reading.date)
            .fontSize(Type.text4)
            .fontColor($r('app.color.fg3'))
            .maxLines(1)
            .padding({ bottom: Metrics.hairline })
        }
      }
      .alignItems(VerticalAlign.Bottom)
      .layoutWeight(1)
      this.trailing()
    }
    .padding({ left: Metrics.s2, right: Metrics.s2 })
    .width('100%')
  }
}

@ComponentV2
export struct UsageTilesCard {
  @Param items: TileItem[] = [];
  @Param wide: boolean = false;

  @Builder
  cell(item: TileItem) {
    Column({ space: ListMetrics.s1_5 }) {
      Text(item.caption)
        .fontSize(Type.text4)
        .fontColor($r('app.color.fg3'))
        .maxLines(1)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
      Row({ space: ListMetrics.s0_5 }) {
        Text(item.value)
          .fontSize(Type.usageFigure)
          .fontWeight(FontWeight.Bold)
          .fontColor($r('app.color.fg1'))
          .maxLines(1)
        if (item.unit !== undefined) {
          Text(item.unit)
            .fontSize(Type.text3)
            .fontColor($r('app.color.fg2'))
            .maxLines(1)
            .padding({ bottom: ListMetrics.s0_5 })
        }
      }
      .alignItems(VerticalAlign.Bottom)
    }
    .alignItems(HorizontalAlign.Start)
    .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s3, bottom: Metrics.s3 })
    .layoutWeight(1)
  }

  build() {
    Column() {
      if (this.items.length >= 4) {
        if (this.wide) {
          Row() {
            this.cell(this.items[0])
            Divider().vertical(true).color($r('app.color.line')).strokeWidth(Metrics.hairline).height(Metrics.barCtl)
            this.cell(this.items[1])
            Divider().vertical(true).color($r('app.color.line')).strokeWidth(Metrics.hairline).height(Metrics.barCtl)
            this.cell(this.items[2])
            Divider().vertical(true).color($r('app.color.line')).strokeWidth(Metrics.hairline).height(Metrics.barCtl)
            this.cell(this.items[3])
          }
          .width('100%')
        } else {
          Row() {
            this.cell(this.items[0])
            Divider().vertical(true).color($r('app.color.line')).strokeWidth(Metrics.hairline).height(Metrics.barCtl)
            this.cell(this.items[1])
          }
          .width('100%')
          Divider().color($r('app.color.line')).strokeWidth(Metrics.hairline).margin({ left: Metrics.pad, right: Metrics.pad })
          Row() {
            this.cell(this.items[2])
            Divider().vertical(true).color($r('app.color.line')).strokeWidth(Metrics.hairline).height(Metrics.barCtl)
            this.cell(this.items[3])
          }
          .width('100%')
        }
      }
    }
    .width('100%')
    .borderRadius(FormMetrics.radius)
    .backgroundColor($r('app.color.form_row'))
    .clip(true)
  }
}
