import { UsageView } from '../wire/Wire';
import { FormMetrics, ListMetrics, Metrics, Type, UsageMetrics } from '../design/Tokens';
import { AppModel } from '../store/AppModel';
import { t } from '../store/L10n';
import { folderLabel, projectName, remoteAlias } from '../store/SessionLogic';
import {
  addDays, addHours, CHATS_KEY, currentRun, dayLabel, dayOf, daysOf, DOT_STEPS, duration, figure, firstDay, hourOf,
  hoursOf, KEEP_ROWS, modelLabels, mondayOffset, monthLong, monthShort, numericDate, percent, periodStart, spaced,
  summarize, TREND_SPAN_HOURS, trendColumns, UsageDay, UsageDim, UsageFigure, UsageModelLabels, UsagePart, UsagePeriod,
  UsageSummary, UsageTrendColumns, weekRange
} from '../logic/UsageStats';
import { AgentMark, Lucide } from '../components/Glyphs';
import { Lead } from '../components/ChatBar';
import { FormPage } from './Settings';

// Settings → Usage (packages/ui/usage/UsagePage.tsx, apps/ios Settings/UsagePage.swift, apps/android ui/UsageScreen.kt):
// four all-time summary tiles, an activity dot lattice over the whole history, a period-switched trend bar chart, and a
// breakdown card by model, agent or project. Both charts paint alpha coverage on a Canvas masking `$r('app.color.fg1')`
// via BlendMode.SRC_IN (as components/Rows.ets `Orb` does) so dark and light themes follow automatically.

class TileItem {
  caption: string = '';
  value: string = '';
  unit: string | undefined = undefined;
}

class SegChoice {
  value: string = '';
  label: string = '';
}

class AxisTick {
  x: number = 0;
  label: string = '';
  /** -1 left edge, 0 centred, 1 right edge */
  edge: number = 0;
}

class TrendCol {
  start: number = 0;
  end: number = 0;
  total: number = 0;
  part: number = 0;
}

class UsageReading {
  tokens: number = 0;
  date: string | undefined = undefined;
  topModel: string | undefined = undefined;
  activeDays: number | undefined = undefined;
}

function makeTile(caption: string, value: string, unit: string | undefined): TileItem {
  const item = new TileItem();
  item.caption = caption;
  item.value = value;
  item.unit = unit;
  return item;
}

function makeSeg(value: string, label: string): SegChoice {
  const c = new SegChoice();
  c.value = value;
  c.label = label;
  return c;
}

function makeTick(x: number, label: string, edge: number): AxisTick {
  const tk = new AxisTick();
  tk.x = x;
  tk.label = label;
  tk.edge = edge;
  return tk;
}

function roundRectPath(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number): void {
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

function topRoundBarPath(ctx: CanvasRenderingContext2D, x: number, yTop: number, w: number, h: number, r: number): void {
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
struct UsageSegmented {
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
struct UsageSectionHead {
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
struct UsageTilesCard {
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

@ComponentV2
struct UsageYearDots {
  @Param days: Map<number, UsageDay> = new Map<number, UsageDay>();
  @Param today: number = 0;
  @Param dim: UsageDim = 'model';
  @Param focusedKey: string = '';
  @Param labels: UsageModelLabels = new UsageModelLabels();
  @Param chartW: number = 312;
  @Local inspected: number = -1;

  private settings: RenderingContextSettings = new RenderingContextSettings(true);
  private ctx: CanvasRenderingContext2D = new CanvasRenderingContext2D(this.settings);
  private ready: boolean = false;

  @Monitor('days', 'today', 'dim', 'focusedKey', 'chartW', 'inspected')
  onRedraw(): void {
    if (this.ready) {
      this.draw();
    }
  }

  private dayValue(d: UsageDay | undefined): number {
    if (d === undefined) {
      return 0;
    }
    return this.focusedKey.length > 0 ? d.tokensFor(this.dim, this.focusedKey) : d.tokens;
  }

  private colsCount(): number {
    return Math.max(8, Math.min(53, Math.floor(this.chartW / UsageMetrics.dotPitchMin)));
  }

  private gap(): number {
    const cols = this.colsCount();
    return Math.max(UsageMetrics.dotGapMin, Math.round((this.chartW / cols) * 0.2));
  }

  private pitch(): number {
    const cols = this.colsCount();
    return (this.chartW + this.gap()) / cols;
  }

  private cellSize(): number {
    return Math.max(1, this.pitch() - this.gap());
  }

  private latticeH(): number {
    return 7 * this.cellSize() + 6 * this.gap();
  }

  private startDay(): number {
    const lastMonday = addDays(this.today, -mondayOffset(this.today));
    return addDays(lastMonday, -(this.colsCount() - 1) * 7);
  }

  private reading(): UsageReading {
    const r = new UsageReading();
    if (this.inspected >= 0) {
      const entry = this.days.get(this.inspected);
      r.tokens = this.dayValue(entry);
      r.date = dayLabel(this.inspected);
      if (this.focusedKey.length === 0 && entry !== undefined) {
        let bestKey = '';
        let bestVal = 0;
        entry.byModel.forEach((v: number, k: string) => {
          if (k.length > 0 && v > bestVal) {
            bestVal = v;
            bestKey = k;
          }
        });
        if (bestKey.length > 0) {
          r.topModel = this.labels.label.get(bestKey)?.name ?? bestKey;
        }
      }
      return r;
    }
    let active = 0;
    this.days.forEach((d: UsageDay) => {
      if (this.dayValue(d) > 0) {
        active++;
      }
    });
    r.activeDays = active;
    return r;
  }

  private months(): AxisTick[] {
    const cols = this.colsCount();
    const start = this.startDay();
    const pitch = this.pitch();
    const out: AxisTick[] = [];
    for (let c = 0; c < cols; c++) {
      const weekEnd = addDays(start, c * 7 + 6);
      if (new Date(weekEnd).getDate() > 7) {
        continue;
      }
      const x = c * pitch;
      if (out.length > 0 && x - out[out.length - 1].x < UsageMetrics.monthGap) {
        continue;
      }
      const edge = x < UsageMetrics.monthEdge ? -1 : x > this.chartW - UsageMetrics.monthEdge ? 1 : 0;
      out.push(makeTick(x, monthShort(weekEnd), edge));
    }
    return out;
  }

  private hitDay(x: number, y: number): number {
    const pitch = this.pitch();
    const g = this.gap();
    const cols = this.colsCount();
    const c = Math.floor((x + g / 2) / pitch);
    const k = Math.floor((y + g / 2) / pitch);
    if (c < 0 || c >= cols || k < 0 || k >= 7) {
      return -1;
    }
    const day = addDays(this.startDay(), c * 7 + k);
    return day <= this.today ? day : -1;
  }

  private draw(): void {
    const w = this.chartW;
    const h = this.latticeH();
    this.ctx.clearRect(0, 0, w, h);
    const cols = this.colsCount();
    const pitch = this.pitch();
    const sz = this.cellSize();
    const corner = Math.min(UsageMetrics.dotCornerMax, sz / 4);
    const start = this.startDay();
    let maxVal = 1;
    this.days.forEach((d: UsageDay) => {
      if (d.day >= start) {
        maxVal = Math.max(maxVal, this.dayValue(d));
      }
    });
    for (let c = 0; c < cols; c++) {
      const x = c * pitch;
      for (let k = 0; k < 7; k++) {
        const day = addDays(start, c * 7 + k);
        if (day > this.today) {
          continue;
        }
        const y = k * pitch;
        const v = this.dayValue(this.days.get(day));
        if (v > 0) {
          const step = Math.max(1, Math.ceil(Math.sqrt(v / maxVal) * DOT_STEPS)) / DOT_STEPS;
          this.ctx.fillStyle = `rgba(0,0,0,${(0.76 * step).toFixed(3)})`;
        } else {
          this.ctx.fillStyle = 'rgba(0,0,0,0.04)';
        }
        roundRectPath(this.ctx, x, y, sz, sz, corner);
        this.ctx.fill();
        if (this.inspected === day) {
          this.ctx.strokeStyle = 'rgba(0,0,0,0.65)';
          this.ctx.lineWidth = Metrics.hairline;
          const o = UsageMetrics.focusOutset;
          roundRectPath(this.ctx, x - o, y - o, sz + o * 2, sz + o * 2, corner + o);
          this.ctx.stroke();
        }
      }
    }
  }

  build() {
    Column({ space: UsageMetrics.groupHeadGap }) {
      UsageSectionHead({ title: t('usagePage.activity'), reading: this.reading() })
      Column({ space: Metrics.s2 }) {
        Stack() {
          Canvas(this.ctx)
            .width(this.chartW)
            .height(this.latticeH())
            .onReady(() => {
              this.ready = true;
              this.draw();
            })
          Column()
            .width(this.chartW)
            .height(this.latticeH())
            .backgroundColor($r('app.color.fg1'))
            .blendMode(BlendMode.SRC_IN)
        }
        .width(this.chartW)
        .height(this.latticeH())
        .blendMode(BlendMode.SRC_OVER, BlendApplyType.OFFSCREEN)
        .onTouch((e: TouchEvent) => {
          if (e.touches.length === 0) {
            return;
          }
          const pt = e.touches[0];
          const hit = this.hitDay(pt.x, pt.y);
          if (e.type === TouchType.Down) {
            this.inspected = hit >= 0 && hit === this.inspected ? -1 : hit;
          } else if (e.type === TouchType.Move && hit >= 0) {
            this.inspected = hit;
          }
        })

        Stack({ alignContent: Alignment.TopStart }) {
          ForEach(this.months(), (m: AxisTick) => {
            Text(m.label)
              .fontSize(Type.text4)
              .fontColor($r('app.color.fg3'))
              .maxLines(1)
              .position({
                x: m.edge === -1 ? 0 : m.edge === 1 ? Math.max(0, this.chartW - UsageMetrics.monthEdge * 2) :
                  Math.max(0, Math.min(this.chartW - UsageMetrics.monthEdge * 2, m.x - UsageMetrics.monthEdge)),
                y: 0
              })
          }, (m: AxisTick) => `${m.x}:${m.label}`)
        }
        .width(this.chartW)
        .height(UsageMetrics.axisH)
      }
      .padding({ left: Metrics.s2, right: Metrics.s2 })
      .width('100%')
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
  }
}

@ComponentV2
struct UsageTrend {
  @Param days: Map<number, UsageDay> = new Map<number, UsageDay>();
  @Param hours: Map<number, UsageDay> = new Map<number, UsageDay>();
  @Param today: number = 0;
  @Param nowHour: number = 0;
  @Param period: UsagePeriod = '30';
  @Param dim: UsageDim = 'model';
  @Param focusedKey: string = '';
  @Param labels: UsageModelLabels = new UsageModelLabels();
  @Param chartW: number = 312;
  @Event onPeriod: (p: UsagePeriod) => void = () => {};
  @Local inspected: number = -1;

  private settings: RenderingContextSettings = new RenderingContextSettings(true);
  private ctx: CanvasRenderingContext2D = new CanvasRenderingContext2D(this.settings);
  private ready: boolean = false;

  @Monitor('days', 'hours', 'today', 'nowHour', 'period', 'dim', 'focusedKey', 'chartW', 'inspected')
  onRedraw(): void {
    if (this.ready) {
      this.draw();
    }
  }

  private layout(): UsageTrendColumns {
    return trendColumns(this.period, this.today, firstDay(this.days, this.today), this.nowHour);
  }

  private cols(l: UsageTrendColumns): TrendCol[] {
    const fold = l.step === 'hours' ? this.hours : this.days;
    const out: TrendCol[] = [];
    for (let i = 0; i < l.cols.length; i++) {
      const span = l.cols[i];
      const col = new TrendCol();
      col.start = span.start;
      col.end = span.end;
      let d = span.start;
      while (d < span.end) {
        const u = fold.get(d);
        if (u !== undefined) {
          col.total += u.tokens;
          if (this.focusedKey.length > 0) {
            col.part += u.tokensFor(this.dim, this.focusedKey);
          }
        }
        const next = l.step === 'hours' ? addHours(d, 1) : addDays(d, 1);
        if (next <= d) {
          break;
        }
        d = next;
      }
      out.push(col);
    }
    return out;
  }

  private periodChoices(): SegChoice[] {
    return [
      makeSeg('7', t('usagePage.period.days', new Map<string, string>([['n', '7']]))),
      makeSeg('30', t('usagePage.period.days', new Map<string, string>([['n', '30']]))),
      makeSeg('all', t('usagePage.period.all'))
    ];
  }

  private reading(): UsageReading {
    const l = this.layout();
    const cols = this.cols(l);
    const r = new UsageReading();
    const filtering = this.focusedKey.length > 0;
    if (this.inspected >= 0 && this.inspected < cols.length) {
      const c = cols[this.inspected];
      r.tokens = filtering ? c.part : c.total;
      if (l.step === 'hours') {
        r.date = `${dayLabel(c.start)} ${new Date(c.start).getHours()}:00`;
      } else if (l.step === 'day') {
        r.date = dayLabel(c.start);
      } else if (l.step === 'week') {
        r.date = weekRange(c.start, addDays(c.end, -1));
      } else {
        r.date = monthLong(c.start);
      }
      if (!filtering) {
        const fold = l.step === 'hours' ? this.hours : this.days;
        const byModel = new Map<string, number>();
        let d = c.start;
        while (d < c.end) {
          fold.get(d)?.byModel.forEach((v: number, k: string) => {
            if (k.length > 0) {
              byModel.set(k, (byModel.get(k) ?? 0) + v);
            }
          });
          const next = l.step === 'hours' ? addHours(d, 1) : addDays(d, 1);
          if (next <= d) {
            break;
          }
          d = next;
        }
        let bestKey = '';
        let bestVal = 0;
        byModel.forEach((v: number, k: string) => {
          if (v > bestVal) {
            bestVal = v;
            bestKey = k;
          }
        });
        if (bestKey.length > 0) {
          r.topModel = this.labels.label.get(bestKey)?.name ?? bestKey;
        }
      }
      return r;
    }
    let sum = 0;
    for (let i = 0; i < cols.length; i++) {
      sum += filtering ? cols[i].part : cols[i].total;
    }
    r.tokens = sum;
    return r;
  }

  private ticks(): AxisTick[] {
    const l = this.layout();
    const cols = this.cols(l);
    const n = cols.length;
    if (n === 0) {
      return [];
    }
    const bw = Math.max(UsageMetrics.barMin, Math.min(UsageMetrics.barMax, (this.chartW / Math.max(1, n)) * 0.64));
    const pitch = n > 1 ? (this.chartW - bw) / (n - 1) : 0;
    const colX = (i: number): number => (n > 1 ? i * pitch : (this.chartW - bw) / 2);
    const labelOf = (ms: number): string => (l.step === 'month' ? monthShort(ms) : numericDate(ms));
    const out: AxisTick[] = [];
    if (l.step === 'hours') {
      const perDay = 24 / TREND_SPAN_HOURS;
      for (let i = 0; i < n; i++) {
        if (new Date(cols[i].start).getHours() === 0) {
          const mid = (colX(i) + colX(Math.min(i + perDay - 1, n - 1)) + bw) / 2;
          out.push(makeTick(mid, labelOf(cols[i].start), 0));
        }
      }
    } else {
      const every = l.step === 'day'
        ? (this.chartW / Math.max(1, n) >= UsageMetrics.labelRoom ? 1 : 7)
        : Math.max(1, Math.ceil((n * UsageMetrics.labelRoom) / Math.max(1, this.chartW)));
      for (let i = n - 1; i >= 0; i -= every) {
        const edge = n > 1 && i === 0 ? -1 : n > 1 && i === n - 1 ? 1 : 0;
        out.push(makeTick(colX(i) + bw / 2, labelOf(cols[i].start), edge));
      }
    }
    return out;
  }

  private hitBar(x: number): number {
    const n = this.layout().cols.length;
    if (n === 0) {
      return -1;
    }
    if (n === 1) {
      return 0;
    }
    const bw = Math.max(UsageMetrics.barMin, Math.min(UsageMetrics.barMax, (this.chartW / n) * 0.64));
    const pitch = (this.chartW - bw) / (n - 1);
    return Math.max(0, Math.min(n - 1, Math.round((x - bw / 2) / pitch)));
  }

  private draw(): void {
    const w = this.chartW;
    const h = UsageMetrics.trendH;
    this.ctx.clearRect(0, 0, w, h);
    const l = this.layout();
    const cols = this.cols(l);
    const n = cols.length;
    this.ctx.strokeStyle = 'rgba(0,0,0,0.06)';
    this.ctx.lineWidth = Metrics.hairline;
    this.ctx.beginPath();
    this.ctx.moveTo(0, h - 0.5);
    this.ctx.lineTo(w, h - 0.5);
    this.ctx.stroke();

    if (n === 0) {
      return;
    }
    const bw = Math.max(UsageMetrics.barMin, Math.min(UsageMetrics.barMax, (w / n) * 0.64));
    const pitch = n > 1 ? (w - bw) / (n - 1) : 0;
    const colX = (i: number): number => (n > 1 ? i * pitch : (w - bw) / 2);
    let maxVal = 1;
    for (let i = 0; i < n; i++) {
      maxVal = Math.max(maxVal, cols[i].total);
    }
    const filtering = this.focusedKey.length > 0;
    const safeIdx = this.inspected >= 0 && this.inspected < n ? this.inspected : -1;
    const drawOne = (i: number, v: number, alpha: number): void => {
      if (v <= 0) {
        return;
      }
      const bh = Math.max(UsageMetrics.barMinH, (v / maxVal) * (h - UsageMetrics.barMinH));
      this.ctx.fillStyle = `rgba(0,0,0,${alpha.toFixed(3)})`;
      topRoundBarPath(this.ctx, colX(i), h - bh, bw, bh, UsageMetrics.barRadiusMax);
      this.ctx.fill();
    };
    for (let i = 0; i < n; i++) {
      const c = cols[i];
      if (c.total <= 0) {
        continue;
      }
      const lit = safeIdx >= 0 ? safeIdx === i : !filtering;
      drawOne(i, c.total, safeIdx === i ? 0.76 : lit ? 0.46 : 0.16);
      if (filtering && safeIdx < 0 && c.part > 0) {
        drawOne(i, c.part, 0.76);
      }
    }
  }

  @Builder
  periodControl() {
    UsageSegmented({
      choices: this.periodChoices(),
      selected: this.period,
      onPick: (v: string) => {
        this.inspected = -1;
        this.onPeriod(v as UsagePeriod);
      }
    })
  }

  build() {
    Column({ space: UsageMetrics.groupHeadGap }) {
      UsageSectionHead({
        title: t('usagePage.trend'),
        reading: this.reading(),
        trailing: () => { this.periodControl(); }
      })
      Column({ space: Metrics.s2 }) {
        Stack() {
          Canvas(this.ctx)
            .width(this.chartW)
            .height(UsageMetrics.trendH)
            .onReady(() => {
              this.ready = true;
              this.draw();
            })
          Column()
            .width(this.chartW)
            .height(UsageMetrics.trendH)
            .backgroundColor($r('app.color.fg1'))
            .blendMode(BlendMode.SRC_IN)
        }
        .width(this.chartW)
        .height(UsageMetrics.trendH)
        .blendMode(BlendMode.SRC_OVER, BlendApplyType.OFFSCREEN)
        .onTouch((e: TouchEvent) => {
          if (e.touches.length === 0) {
            return;
          }
          const hit = this.hitBar(e.touches[0].x);
          if (e.type === TouchType.Down) {
            this.inspected = hit >= 0 && hit === this.inspected ? -1 : hit;
          } else if (e.type === TouchType.Move && hit >= 0) {
            this.inspected = hit;
          }
        })

        Stack({ alignContent: Alignment.TopStart }) {
          ForEach(this.ticks(), (tk: AxisTick) => {
            Text(tk.label)
              .fontSize(Type.text4)
              .fontColor($r('app.color.fg3'))
              .maxLines(1)
              .position({
                x: tk.edge === -1 ? 0 : tk.edge === 1 ? Math.max(0, this.chartW - UsageMetrics.monthEdge * 2) :
                  Math.max(0, Math.min(this.chartW - UsageMetrics.monthEdge * 2, tk.x - UsageMetrics.monthEdge)),
                y: 0
              })
          }, (tk: AxisTick) => `${tk.x}:${tk.label}`)
        }
        .width(this.chartW)
        .height(UsageMetrics.axisH)
      }
      .padding({ left: Metrics.s2, right: Metrics.s2 })
      .width('100%')
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
  }
}

@ComponentV2
struct UsageBreakdownCard {
  @Require @Param model: AppModel;
  @Param dim: UsageDim = 'model';
  @Param items: UsagePart[] = [];
  @Param labels: UsageModelLabels = new UsageModelLabels();
  @Param focusedKey: string = '';
  @Param wide: boolean = false;
  @Event onDim: (d: UsageDim) => void = () => {};
  @Event onPickFocus: (key: string) => void = () => {};
  @Local expanded: boolean = false;

  @Monitor('dim')
  onDimChange(): void {
    this.expanded = false;
  }

  private dimChoices(): SegChoice[] {
    return [
      makeSeg('model', t('usagePage.dim.model')),
      makeSeg('agent', t('usagePage.dim.agent')),
      makeSeg('project', t('usagePage.dim.project'))
    ];
  }

  private projectLabel(key: string): string {
    return key === CHATS_KEY ? t('session.group.chats') : folderLabel(key);
  }

  private rowName(key: string): string {
    if (this.dim === 'agent') {
      return this.model.agentName(key);
    }
    if (this.dim === 'project') {
      return this.projectLabel(key);
    }
    return this.labels.label.get(key)?.name ?? key;
  }

  private rowDetail(key: string): string {
    if (this.dim !== 'project' || key === CHATS_KEY) {
      return '';
    }
    const alias = remoteAlias(key);
    if (alias !== undefined) {
      return alias;
    }
    const label = this.projectLabel(key);
    const twin = this.items.some((p: UsagePart) => p.key !== key && p.key !== CHATS_KEY && this.projectLabel(p.key) === label);
    if (!twin) {
      return '';
    }
    const cut = Math.max(key.lastIndexOf('/'), key.lastIndexOf('\\'));
    return cut > 0 ? projectName(key.substring(0, cut)) : '';
  }

  private visibleItems(): UsagePart[] {
    const folds = this.items.length > KEEP_ROWS + 1;
    return !folds || this.expanded ? this.items : this.items.slice(0, KEEP_ROWS);
  }

  private foldedRest(): UsagePart[] {
    return this.items.length > KEEP_ROWS + 1 ? this.items.slice(KEEP_ROWS) : [];
  }

  private othersPart(): UsagePart {
    const rest = this.foldedRest();
    const out = new UsagePart();
    out.key = '*';
    for (let i = 0; i < rest.length; i++) {
      out.tokens += rest[i].tokens;
      out.share += rest[i].share;
    }
    return out;
  }

  @Builder
  dimControl() {
    UsageSegmented({
      choices: this.dimChoices(),
      selected: this.dim,
      onPick: (v: string) => this.onDim(v as UsageDim)
    })
  }

  @Builder
  markSlot(key: string, isMore: boolean) {
    Row() {
      if (isMore) {
        Lucide({ name: 'ellipsis', tint: $r('app.color.fg3'), extent: Metrics.icon })
      } else if (this.dim === 'model') {
        AgentMark({ agent: this.labels.label.get(key)?.brand ?? '', name: this.rowName(key), tint: $r('app.color.fg2'), extent: Metrics.icon })
      } else if (this.dim === 'agent') {
        AgentMark({ agent: key, name: this.rowName(key), tint: $r('app.color.fg2'), extent: Metrics.icon })
      } else if (key === CHATS_KEY) {
        Lucide({ name: 'messageSquare', tint: $r('app.color.fg2'), extent: Metrics.icon })
      } else if (remoteAlias(key) !== undefined) {
        Stack({ alignContent: Alignment.BottomEnd }) {
          Lucide({ name: 'folder', tint: $r('app.color.fg2'), extent: Metrics.icon })
          Lucide({ name: 'globe', tint: $r('app.color.fg2'), extent: UsageMetrics.crown })
        }
        .width(Metrics.icon)
        .height(Metrics.icon)
      } else {
        Lucide({ name: 'folder', tint: $r('app.color.fg2'), extent: Metrics.icon })
      }
    }
    .width(Metrics.lead)
    .height(Metrics.lead)
    .justifyContent(FlexAlign.Center)
  }

  @Builder
  shareTrack(share: number, muted: boolean) {
    Stack({ alignContent: Alignment.Start }) {
      Row()
        .width('100%')
        .height(UsageMetrics.trackH)
        .borderRadius(UsageMetrics.trackH / 2)
        .backgroundColor($r('app.color.usage_track'))
      Row()
        .width(`${Math.max(0, Math.min(100, share * 100))}%`)
        .height(UsageMetrics.trackH)
        .borderRadius(UsageMetrics.trackH / 2)
        .backgroundColor(muted ? $r('app.color.usage_dim') : $r('app.color.usage_ink'))
    }
    .height(UsageMetrics.trackH)
    .width('100%')
  }

  @Builder
  tokenReadout(fig: UsageFigure) {
    Row({ space: ListMetrics.s0_5 }) {
      Text(fig.value)
        .fontSize(Type.text3)
        .fontColor($r('app.color.fg1'))
        .maxLines(1)
      if (fig.unit !== undefined) {
        Text(fig.unit)
          .fontSize(Type.text4)
          .fontColor($r('app.color.fg3'))
          .maxLines(1)
      }
    }
    .justifyContent(FlexAlign.End)
    .alignItems(VerticalAlign.Bottom)
  }

  @Builder
  breakdownRow(part: UsagePart, isMore: boolean, moreCount: number) {
    Row({ space: Metrics.gap }) {
      this.markSlot(part.key, isMore)
      if (this.wide) {
        Row({ space: ListMetrics.s1_5 }) {
          Text(isMore ? t(`usagePage.more.${this.dim}`, new Map<string, string>([['n', `${moreCount}`]])) : this.rowName(part.key))
            .fontSize(Type.text3)
            .fontColor(isMore ? $r('app.color.fg2') : $r('app.color.fg1'))
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
            .flexShrink(1)
          if (!isMore && this.rowDetail(part.key).length > 0) {
            Text(this.rowDetail(part.key))
              .fontSize(Type.text4)
              .fontColor($r('app.color.fg3'))
              .maxLines(1)
          }
        }
        .width(UsageMetrics.nameWide)
        Row() {
          this.shareTrack(part.share, isMore)
        }
        .layoutWeight(1)
        Row() {
          this.tokenReadout(figure(part.tokens))
        }
        .width(UsageMetrics.numW)
        .justifyContent(FlexAlign.End)
        Text(percent(part.share))
          .fontSize(Type.text4)
          .fontColor($r('app.color.fg3'))
          .textAlign(TextAlign.End)
          .width(UsageMetrics.pctW)
          .maxLines(1)
      } else {
        Column({ space: ListMetrics.s1_5 }) {
          Row({ space: Metrics.s2 }) {
            Row({ space: ListMetrics.s1_5 }) {
              Text(isMore ? t(`usagePage.more.${this.dim}`, new Map<string, string>([['n', `${moreCount}`]])) : this.rowName(part.key))
                .fontSize(Type.text3)
                .fontColor(isMore ? $r('app.color.fg2') : $r('app.color.fg1'))
                .maxLines(1)
                .textOverflow({ overflow: TextOverflow.Ellipsis })
                .flexShrink(1)
              if (!isMore && this.rowDetail(part.key).length > 0) {
                Text(this.rowDetail(part.key))
                  .fontSize(Type.text4)
                  .fontColor($r('app.color.fg3'))
                  .maxLines(1)
              }
            }
            .layoutWeight(1)
            this.tokenReadout(figure(part.tokens))
            Text(percent(part.share))
              .fontSize(Type.text4)
              .fontColor($r('app.color.fg3'))
              .textAlign(TextAlign.End)
              .width(UsageMetrics.pctW)
              .maxLines(1)
          }
          .width('100%')
          this.shareTrack(part.share, isMore)
        }
        .layoutWeight(1)
      }
    }
    .width('100%')
    .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s3, bottom: Metrics.s3 })
    .opacity(this.focusedKey.length > 0 && this.focusedKey !== part.key ? UsageMetrics.dimRow : 1)
    .onClick(() => {
      if (isMore) {
        this.onPickFocus('');
        this.expanded = true;
      } else {
        this.onPickFocus(this.focusedKey === part.key ? '' : part.key);
      }
    })
  }

  build() {
    Column({ space: UsageMetrics.groupHeadGap }) {
      UsageSectionHead({
        title: t('usagePage.breakdown'),
        count: this.items.length,
        trailing: () => { this.dimControl(); }
      })
      if (this.items.length > 0) {
        Column() {
          ForEach(this.visibleItems(), (part: UsagePart, idx: number) => {
            Column() {
              if (idx > 0) {
                Divider().color($r('app.color.line')).strokeWidth(Metrics.hairline).margin({ left: Metrics.pad, right: Metrics.pad })
              }
              this.breakdownRow(part, false, 0)
            }
            .width('100%')
          }, (part: UsagePart) => `${part.key}:${part.tokens}`)
          if (this.foldedRest().length > 0 && !this.expanded) {
            Divider().color($r('app.color.line')).strokeWidth(Metrics.hairline).margin({ left: Metrics.pad, right: Metrics.pad })
            this.breakdownRow(this.othersPart(), true, this.foldedRest().length)
          }
        }
        .width('100%')
        .borderRadius(FormMetrics.radius)
        .backgroundColor($r('app.color.form_row'))
        .clip(true)
      }
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
  }
}

/** Settings → Usage page */
@ComponentV2
export struct UsagePage {
  @Require @Param model: AppModel;
  @Param lead: Lead = Lead.Back;
  @Event onBack: () => void = () => {};
  @Local period: UsagePeriod = '30';
  @Local dim: UsageDim = 'model';
  @Local focusedKey: string = '';
  @Local contentW: number = 328;

  aboutToAppear(): void {
    this.model.loadUsage();
  }

  private tiles(all: UsageSummary, streak: number): TileItem[] {
    const tokens = figure(all.tokens);
    const work = duration(all.workMs);
    const longest = duration(all.longestMs);
    return [
      makeTile(t('usagePage.fig.tokens'), tokens.value, tokens.unit),
      makeTile(t('usagePage.fig.work'), work.value, work.unit),
      makeTile(t('usagePage.fig.longest'), longest.value, longest.unit),
      makeTile(t('usagePage.fig.streak'), `${streak}`, t('usagePage.unit.days'))
    ];
  }

  @Builder
  body(view: UsageView) {
    UsageBody({
      model: this.model,
      view: view,
      period: this.period,
      dim: this.dim,
      focusedKey: this.focusedKey,
      contentW: this.contentW,
      onPeriod: (p: UsagePeriod) => { this.period = p; },
      onDim: (d: UsageDim) => {
        this.dim = d;
        this.focusedKey = '';
      },
      onPickFocus: (k: string) => { this.focusedKey = k; }
    })
  }

  build() {
    FormPage({ title: t('settings.nav.usage'), lead: this.lead, onBack: () => this.onBack() }) {
      Column({ space: UsageMetrics.sectionGap }) {
        if (this.model.usageView !== undefined) {
          this.body(this.model.usageView)
        } else {
          Row() {
            Text(t('settings.loading'))
              .fontSize(Type.text3)
              .fontColor($r('app.color.fg3'))
          }
          .padding(Metrics.pad)
          .width('100%')
          .borderRadius(FormMetrics.radius)
          .backgroundColor($r('app.color.form_row'))
        }
      }
      .width('100%')
      .onAreaChange((_old: Area, next: Area) => {
        const w = Number(next.width);
        if (w > 0 && Math.abs(w - this.contentW) >= 1) {
          this.contentW = w;
        }
      })
    }
  }
}

@ComponentV2
struct UsageBody {
  @Require @Param model: AppModel;
  @Require @Param view: UsageView;
  @Param period: UsagePeriod = '30';
  @Param dim: UsageDim = 'model';
  @Param focusedKey: string = '';
  @Param contentW: number = 328;
  @Event onPeriod: (p: UsagePeriod) => void = () => {};
  @Event onDim: (d: UsageDim) => void = () => {};
  @Event onPickFocus: (k: string) => void = () => {};

  build() {
    Column({ space: UsageMetrics.sectionGap }) {
      UsageDerived({
        model: this.model,
        view: this.view,
        period: this.period,
        dim: this.dim,
        focusedKey: this.focusedKey,
        contentW: this.contentW,
        onPeriod: (p: UsagePeriod) => this.onPeriod(p),
        onDim: (d: UsageDim) => this.onDim(d),
        onPickFocus: (k: string) => this.onPickFocus(k)
      })
    }
    .width('100%')
  }
}

@ComponentV2
struct UsageDerived {
  @Require @Param model: AppModel;
  @Require @Param view: UsageView;
  @Param period: UsagePeriod = '30';
  @Param dim: UsageDim = 'model';
  @Param focusedKey: string = '';
  @Param contentW: number = 328;
  @Event onPeriod: (p: UsagePeriod) => void = () => {};
  @Event onDim: (d: UsageDim) => void = () => {};
  @Event onPickFocus: (k: string) => void = () => {};

  @Local labels: UsageModelLabels = new UsageModelLabels();
  @Local days: Map<number, UsageDay> = new Map<number, UsageDay>();
  @Local hours: Map<number, UsageDay> = new Map<number, UsageDay>();
  @Local all: UsageSummary = new UsageSummary();
  @Local current: UsageSummary = new UsageSummary();
  @Local today: number = 0;
  @Local nowHour: number = 0;
  @Local streak: number = 0;

  aboutToAppear(): void {
    this.recompute();
  }

  @Monitor('view', 'period')
  recompute(): void {
    const nowMs = this.model.now();
    this.today = dayOf(nowMs);
    this.nowHour = hourOf(nowMs);
    this.labels = modelLabels(this.view, this.model.chatRoot);
    this.days = daysOf(this.view, this.labels);
    this.hours = hoursOf(this.view, this.labels);
    this.all = summarize(this.view, this.labels, 0);
    this.current = summarize(this.view, this.labels, periodStart(this.period, this.today));
    this.streak = currentRun(this.days, this.today);
  }

  private tiles(): TileItem[] {
    const tokens = figure(this.all.tokens);
    const work = duration(this.all.workMs);
    const longest = duration(this.all.longestMs);
    return [
      makeTile(t('usagePage.fig.tokens'), tokens.value, tokens.unit),
      makeTile(t('usagePage.fig.work'), work.value, work.unit),
      makeTile(t('usagePage.fig.longest'), longest.value, longest.unit),
      makeTile(t('usagePage.fig.streak'), `${this.streak}`, t('usagePage.unit.days'))
    ];
  }

  build() {
    Column({ space: UsageMetrics.sectionGap }) {
      UsageTilesCard({ items: this.tiles(), wide: this.contentW >= UsageMetrics.tilesWide })
      UsageYearDots({
        days: this.days,
        today: this.today,
        dim: this.dim,
        focusedKey: this.focusedKey,
        labels: this.labels,
        chartW: Math.max(120, this.contentW - Metrics.s2 * 2)
      })
      UsageTrend({
        days: this.days,
        hours: this.hours,
        today: this.today,
        nowHour: this.nowHour,
        period: this.period,
        dim: this.dim,
        focusedKey: this.focusedKey,
        labels: this.labels,
        chartW: Math.max(120, this.contentW - Metrics.s2 * 2),
        onPeriod: (p: UsagePeriod) => this.onPeriod(p)
      })
      UsageBreakdownCard({
        model: this.model,
        dim: this.dim,
        items: this.current.parts(this.dim),
        labels: this.labels,
        focusedKey: this.focusedKey,
        wide: this.contentW >= UsageMetrics.tilesWide,
        onDim: (d: UsageDim) => this.onDim(d),
        onPickFocus: (k: string) => this.onPickFocus(k)
      })
    }
    .width('100%')
  }
}
