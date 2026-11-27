import { UsageView } from '../wire/Wire';
import { FormMetrics, Metrics, Type, UsageMetrics } from '../design/Tokens';
import { AppModel } from '../store/AppModel';
import { t } from '../store/L10n';
import {
  currentRun, dayOf, daysOf, duration, figure, hourOf, hoursOf, modelLabels, periodStart, summarize, UsageDay,
  UsageDim, UsageModelLabels, UsagePeriod, UsageSummary
} from '../logic/UsageStats';
import { Lead } from '../components/ChatBar';
import { FormPage } from './Settings';
import { makeTile, TileItem, UsageTilesCard } from '../components/usage/UsageCards';
import { UsageTrend, UsageYearDots } from '../components/usage/UsageCharts';
import { UsageBreakdownCard } from '../components/usage/UsageBreakdownCard';

// Settings → Usage (packages/ui/usage/UsagePage.tsx, apps/ios Settings/UsagePage.swift, apps/android ui/UsageScreen.kt):
// four all-time summary tiles, an activity dot lattice over the whole history, a period-switched trend bar chart, and a
// breakdown card by model, agent or project. Both charts paint alpha coverage on a Canvas masking `$r('app.color.fg1')`
// via BlendMode.SRC_IN (as components/Rows.ets `Orb` does) so dark and light themes follow automatically.

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
