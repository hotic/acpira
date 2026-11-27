import { Metrics, Type, UsageMetrics } from '../../design/Tokens';
import { t } from '../../store/L10n';
import {
  addDays, addHours, dayLabel, DOT_STEPS, firstDay, mondayOffset, monthLong, monthShort, numericDate,
  TREND_SPAN_HOURS, trendColumns, UsageDay, UsageDim, UsageModelLabels, UsagePeriod, UsageTrendColumns, weekRange
} from '../../logic/UsageStats';
import {
  AxisTick, makeSeg, makeTick, roundRectPath, SegChoice, topRoundBarPath, TrendCol, UsageReading, UsageSectionHead,
  UsageSegmented
} from './UsageCards';

// Settings → Usage's two charts (pages/Usage.ets): the activity dot lattice over the whole history and the
// period-switched trend

@ComponentV2
export struct UsageYearDots {
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
export struct UsageTrend {
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
