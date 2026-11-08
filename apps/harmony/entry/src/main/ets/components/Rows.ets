import { Metrics, WebMetrics, WebType } from '../design/Tokens';
import { Lucide } from './Glyphs';
import { t } from '../store/L10n';

// ui/Row.tsx, ui/Disclosure.tsx and ui/ConnectedRail.tsx as the iOS app ports them (Components/Row.swift,
// Disclosure.swift, Rail.swift; apps/android ui/Rows.kt): the one row every thought / plan / tool / status grows on, the
// fold that reveals a body under it, and the rail drawn from a lead icon down an open body or through result rows.

// Fold state

/**
 * The reader's fold choices, kept outside the rows (FoldMemory): a turn's rows are built again whenever a patch changes
 * them, and a choice must survive that. `all` opens every fold (the Previewer's fixture pages, which take no taps)
 */
@ObservedV2
export class FoldStore {
  @Trace open: Map<string, boolean> = new Map<string, boolean>();
  @Trace all: boolean = false;

  isOpen(key: string, fallback: boolean): boolean {
    if (this.all) {
      return true;
    }
    return this.open.get(key) ?? fallback;
  }

  toggle(ui: UIContext, key: string, fallback: boolean): void {
    const next = !this.isOpen(key, fallback);
    ui.animateTo({ duration: WebMetrics.foldMs, curve: Curve.Smooth }, () => {
      this.open.set(key, next);
    });
  }
}

export const folds: FoldStore = new FoldStore();

/**
 * One clock for every live row: whole-second ticks for running times and the shimmer's breath. A live turn's rows are
 * built again on every patch, so a row-owned timer or animation would restart with each chunk and never get far
 */
@ObservedV2
export class Ticker {
  @Trace tick: number = 0;
  @Trace bright: boolean = false;
  private clock: number = -1;
  private breath: number = -1;

  start(): void {
    if (this.clock < 0) {
      this.clock = setInterval(() => {
        this.tick++;
      }, WebMetrics.clockTick);
    }
    if (this.breath < 0) {
      this.breath = setInterval(() => {
        this.bright = !this.bright;
      }, WebMetrics.shimmerCycle / 2);
    }
  }
}

export const ticker: Ticker = new Ticker();

/** The Orb's phase runs from one start for the whole app, so a rebuilt row picks the ring up where it was */
const orbEpoch: number = Date.now();

// Row

/**
 * What a row shows (Row.tsx's slots as data): the lead (an icon, the Orb, a plan status dot or nothing), the label, the
 * live clock or elapsed time beside it, the target with its faint note, a count and a fold mark, then the trailing
 * meta: a diff stat, a caption, a command's ✓ / ✗ and run time
 */
export class RowSpec {
  /** icon, none, orb, plan */
  lead: string = 'icon';
  icon: string = '';
  iconTint: ResourceColor | undefined = undefined;
  /** plan: the entry's status (completed, in_progress, pending) */
  planStatus: string = '';
  /** Hide the rail behind the lead icon (rows a rail runs through) */
  mask: boolean = false;
  label: string = '';
  labelInk: ResourceColor | undefined = undefined;
  labelWeight: FontWeight = FontWeight.Normal;
  /** The label may wrap (notice titles, entry titles); otherwise it keeps one line */
  wrap: boolean = false;
  shimmer: boolean = false;
  /** Faint tabular text right after the label: the fold head's time */
  elapsed: string = '';
  target: string = '';
  targetMono: boolean = false;
  targetInk: ResourceColor | undefined = undefined;
  aside: string = '';
  count: string = '';
  /** '', right (the fold chevron), caret (the plan's count caret) */
  chevron: string = '';
  open: boolean = false;
  add: number = 0;
  del: number = 0;
  meta: string = '';
  metaInk: ResourceColor | undefined = undefined;
  /** A command's end: completed, failed (✓ / ✗), or '' */
  outcome: string = '';
  /** Keeps the outcome's slot while the command runs, so its arrival moves nothing */
  outcomeSlot: boolean = false;
  duration: string = '';
  height: number = WebMetrics.row;
  /** The action tone: verbs and targets in the action ink */
  action: boolean = false;
  /** A live clock after the label: the epoch ms it counts from (0: none) and the clock to read */
  since: number = 0;
  now: () => number = () => Date.now();
  /** A running command's clock in the trailing slot: the epoch ms it counts from (0: none) */
  runSince: number = 0;

  /** The same row with its fold mark set */
  folded(open: boolean): RowSpec {
    const s = new RowSpec();
    s.lead = this.lead;
    s.icon = this.icon;
    s.iconTint = this.iconTint;
    s.planStatus = this.planStatus;
    s.mask = this.mask;
    s.label = this.label;
    s.labelInk = this.labelInk;
    s.labelWeight = this.labelWeight;
    s.wrap = this.wrap;
    s.shimmer = this.shimmer;
    s.elapsed = this.elapsed;
    s.target = this.target;
    s.targetMono = this.targetMono;
    s.targetInk = this.targetInk;
    s.aside = this.aside;
    s.count = this.count;
    s.chevron = this.chevron.length > 0 ? this.chevron : 'right';
    s.open = open;
    s.add = this.add;
    s.del = this.del;
    s.meta = this.meta;
    s.metaInk = this.metaInk;
    s.outcome = this.outcome;
    s.outcomeSlot = this.outcomeSlot;
    s.duration = this.duration;
    s.height = this.height;
    s.action = this.action;
    s.since = this.since;
    s.now = this.now;
    s.runSince = this.runSince;
    return s;
  }
}

export function rowSpec(icon: string, label: string, height: number): RowSpec {
  const s = new RowSpec();
  s.icon = icon;
  s.lead = icon.length > 0 ? 'icon' : 'none';
  s.label = label;
  s.height = height;
  return s;
}

/** The plan entry's status: a filled check, a ring around an accent dot, an empty ring */
@ComponentV2
export struct PlanDot {
  @Param status: string = 'pending';

  build() {
    Stack() {
      if (this.status === 'completed') {
        Circle()
          .width(WebMetrics.chevron)
          .height(WebMetrics.chevron)
          .fill($r('app.color.fg3'))
        Lucide({ name: 'check', tint: $r('app.color.bg0'), extent: WebMetrics.planCheck, weight: WebMetrics.strokePlanCheck })
      } else if (this.status === 'in_progress') {
        Circle()
          .width(WebMetrics.chevron)
          .height(WebMetrics.chevron)
          .fill(Color.Transparent)
          .stroke($r('app.color.brand'))
          .strokeWidth(WebMetrics.planRing)
        Circle()
          .width(WebMetrics.planDot)
          .height(WebMetrics.planDot)
          .fill($r('app.color.brand'))
      } else {
        Circle()
          .width(WebMetrics.chevron)
          .height(WebMetrics.chevron)
          .fill(Color.Transparent)
          .stroke($r('app.color.fg3'))
          .strokeWidth(WebMetrics.planRing)
      }
    }
    .width(WebMetrics.chevron)
    .height(WebMetrics.chevron)
  }
}

/** The fold chevron: ChevronRight size-3 turned 90° when open; the caret turns over */
@ComponentV2
export struct FoldMark {
  @Param open: boolean = false;
  @Param caret: boolean = false;
  @Param tint: ResourceColor = $r('app.color.fg2');

  build() {
    Lucide({
      name: this.caret ? 'chevronDown' : 'chevronRight', tint: this.tint,
      extent: this.caret ? WebMetrics.icon : WebMetrics.chevron, weight: this.caret ? WebMetrics.strokeIcon : WebMetrics.strokeChevron
    })
      .rotate({ angle: this.open ? (this.caret ? 180 : 90) : 0 })
      .animation({ duration: WebMetrics.foldMs, curve: Curve.Smooth })
  }
}

/** The working Orb: a breathing ring of dots in the lead slot (the `ring` painter, resolvePreset('breathing', 20)) */
@ComponentV2
export struct Orb {
  private settings: RenderingContextSettings = new RenderingContextSettings(true);
  private ctx: CanvasRenderingContext2D = new CanvasRenderingContext2D(this.settings);
  private timer: number = -1;
  aboutToDisappear(): void {
    if (this.timer >= 0) {
      clearInterval(this.timer);
    }
  }

  draw(): void {
    const n = WebMetrics.lead;
    const s = (Date.now() - orbEpoch) / 1000 * WebMetrics.orbSpeed;
    this.ctx.clearRect(0, 0, n, n);
    // Coverage only: each dot's grey (its lightness against the page) becomes opacity of the page ink laid over it
    for (const d of orbFrame(n, s)) {
      this.ctx.fillStyle = `rgba(0,0,0,${(d.a * (1 - d.white)).toFixed(3)})`;
      this.ctx.beginPath();
      this.ctx.arc(d.x, d.y, d.r, 0, Math.PI * 2);
      this.ctx.fill();
    }
  }

  build() {
    // The dots mask a layer of the page ink, so the ring follows the colour mode without resolving a colour
    Stack() {
      Canvas(this.ctx)
        .width(WebMetrics.lead)
        .height(WebMetrics.lead)
        .onReady(() => {
          this.draw();
          this.timer = setInterval(() => this.draw(), WebMetrics.orbFrame);
        })
      Column()
        .width(WebMetrics.lead)
        .height(WebMetrics.lead)
        .backgroundColor($r('app.color.fg1'))
        .blendMode(BlendMode.SRC_IN)
    }
    .width(WebMetrics.lead)
    .height(WebMetrics.lead)
    .blendMode(BlendMode.SRC_OVER, BlendApplyType.OFFSCREEN)
  }
}

class OrbDot {
  x: number = 0;
  y: number = 0;
  z: number = 0;
  r: number = 0;
  white: number = 0;
  a: number = 0;
}

/** The ring painter's frame, one to one with the iOS app's OrbRing (sizes in vp, painted back to front) */
function orbFrame(n: number, s: number): OrbDot[] {
  const lanes = 2;
  const segs = 15;
  const rBase = 1.7842;
  const rDepth = 2.7574;
  const rsPow = 0.6;
  const rMin = 0.3;
  const bandMul = 3.968;
  const wobMul = 0.565;
  const tilt = 0.3;
  const cx = n / 2;
  const cy = n / 2;
  const o = n / 2 * 0.78;
  const sinT = Math.sin(tilt);
  const cosT = Math.cos(tilt);
  const m = Math.pow(n / 300, rsPow);
  const l = -tilt;
  const y = Math.cos(l);
  const b = Math.sin(l);
  const bigP = -b;
  const x = y;
  const g = 0.23 * wobMul;
  const d = o / (1 + 0.85 * g);
  const bands = Math.max(1, Math.round(lanes * bandMul));
  const half = (bands - 1) / 2;
  const dots: OrbDot[] = [];
  for (let z = 0; z < bands; z++) {
    const bigO = (z - half) * 0.075;
    const bigB = Math.abs(z - half) / Math.max(1, half);
    for (let k = 0; k < segs; k++) {
      const bigS = k / segs * 2 * Math.PI;
      const bigA = (0.16 * Math.sin(bigS * 3 - s * 1.7 + z * 0.22) + 0.07 * Math.sin(bigS * 5 + s * 1.1)) * wobMul;
      const q = Math.cos(bigS);
      const bigF = y * Math.sin(bigS) + bigP * bigO;
      const j = b * Math.sin(bigS) + x * bigO;
      const bigW = Math.sqrt(q * q + bigF * bigF + j * j);
      const bigY = d * (1 + bigA);
      const px0 = q / bigW * bigY;
      const py0 = bigF / bigW * bigY;
      const pz0 = j / bigW * bigY;
      const ry = py0 * cosT - pz0 * sinT;
      const rz = py0 * sinT + pz0 * cosT;
      const bigK = (rz / o + 1) / 2;
      const dot = new OrbDot();
      dot.x = cx + px0;
      dot.y = cy - ry;
      dot.z = rz;
      dot.r = Math.max(rMin, (rBase + rDepth * bigK) * (1 - 0.25 * bigB) * m);
      dot.white = Math.min(1, Math.max(0, 0.52 - 0.44 * bigK + 0.18 * bigB));
      dot.a = 0.4 + 0.6 * bigK;
      if (dot.a >= 0.02) {
        dots.push(dot);
      }
    }
  }
  return dots.sort((p: OrbDot, q: OrbDot) => p.z - q.z);
}

/**
 * A command's ✓ / ✗ in a slot held while it runs (CommandOutcome.tsx), so its arrival moves nothing on the row; it pops
 * in when the command ends on screen
 */
@ComponentV2
export struct OutcomeMark {
  @Param outcome: string = '';

  build() {
    Stack() {
      if (this.outcome === 'completed' || this.outcome === 'failed') {
        Lucide({
          name: this.outcome === 'completed' ? 'check' : 'x',
          tint: this.outcome === 'completed' ? $r('app.color.ok') : $r('app.color.danger'),
          extent: WebMetrics.chevron, weight: WebMetrics.strokeOutcome
        })
          .transition(TransitionEffect.scale({ x: 0.6, y: 0.6 }).combine(TransitionEffect.OPACITY)
            .animation({ duration: WebMetrics.outcomeMs, curve: Curve.Friction }))
      }
    }
    .width(WebMetrics.chevron)
    .height(WebMetrics.chevron)
  }
}

/**
 * ui/Row: the lead slot (its icon centred on an icon-wide column at the row's start, so the ink stands on the text edge
 * above and below it; the slot keeps its width, so the label column stays put), the content parts on one line with an
 * 8 vp gap, trailing meta right-aligned in caption type
 */
@ComponentV2
export struct AcpRow {
  @Param spec: RowSpec = new RowSpec();

  aboutToAppear(): void {
    // A live label (ui/Shimmer) breathes between the base and strong inks on the shared tick: ArkUI text takes no
    // gradient ink, so the webview's sweeping band becomes a breath of the same inks and period
    if (this.spec.shimmer) {
      ticker.start();
    }
  }

  ink(): ResourceColor {
    return this.spec.action ? $r('app.color.action_fg') : $r('app.color.fg2');
  }

  @Builder
  leadSlot() {
    Stack() {
      if (this.spec.lead === 'orb') {
        Orb()
      } else if (this.spec.lead === 'plan') {
        PlanDot({ status: this.spec.planStatus })
      } else if (this.spec.icon.length > 0) {
        if (this.spec.mask) {
          // The rail runs behind result rows: the icon's own square hides it, as the web draws it icon to icon
          Column()
            .width(WebMetrics.icon)
            .height(WebMetrics.icon)
            .backgroundColor($r('app.color.bg0'))
        }
        Lucide({
          name: this.spec.icon, tint: this.spec.iconTint ?? (this.spec.action ? $r('app.color.action_fg') : $r('app.color.fg3')),
          extent: WebMetrics.icon, weight: WebMetrics.strokeIcon
        })
      }
    }
    .width(WebMetrics.lead)
    .height(this.spec.wrap ? WebType.text2Line : WebMetrics.lead)
    .margin({ left: (WebMetrics.icon - WebMetrics.lead) / 2, right: Metrics.gap - (WebMetrics.lead - WebMetrics.icon) / 2 })
  }

  build() {
    Row() {
      if (this.spec.lead !== 'none') {
        this.leadSlot()
      }
      Row({ space: Metrics.s2 }) {
        if (this.spec.label.length > 0) {
          // The shimmer stays inline: a custom component's flexShrink does not reach its Text, so a long target would
          // squeeze the label
          Text(this.spec.label)
            .fontSize(WebType.text2)
            .lineHeight(WebType.text2Line)
            .fontWeight(this.spec.labelWeight)
            .fontColor(this.spec.shimmer ? (ticker.bright ? $r('app.color.fg_strong') : $r('app.color.shimmer_base')) : (this.spec.labelInk ?? this.ink()))
            .animation(this.spec.shimmer ? { duration: WebMetrics.shimmerCycle / 2, curve: Curve.EaseInOut } : undefined)
            .maxLines(this.spec.wrap ? WebMetrics.allLines : 1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
            .flexShrink(this.spec.wrap || this.spec.target.length === 0 ? 1 : 0)
        }
        if (this.spec.elapsed.length > 0) {
          Text(this.spec.elapsed)
            .fontSize(WebType.text2)
            .lineHeight(WebType.text2Line)
            .fontColor($r('app.color.fg3'))
            .fontFeature('"tnum" on')
            .maxLines(1)
            .flexShrink(0)
        }
        if (this.spec.since > 0) {
          LiveClock({ since: this.spec.since, now: this.spec.now })
            .flexShrink(0)
        }
        if (this.spec.target.length > 0) {
          Text(this.spec.target.split('\n').join(' '))
            .fontSize(this.spec.targetMono ? WebType.mono : WebType.text2)
            .lineHeight(WebType.text2Line)
            .fontFamily(this.spec.targetMono ? 'monospace' : 'HarmonyOS Sans')
            .fontColor(this.spec.targetInk ?? (this.spec.action ? $r('app.color.action_fg') : $r('app.color.fg1')))
            .opacity(this.spec.targetInk === undefined && !this.spec.action ? 0.85 : 1)
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
            .flexShrink(1)
        }
        if (this.spec.aside.length > 0) {
          Text(this.spec.aside)
            .fontSize(WebType.text3)
            .lineHeight(WebType.text3Line)
            .fontColor(this.spec.action ? $r('app.color.action_fg') : $r('app.color.fg2'))
            .fontFeature('"tnum" on')
            .maxLines(1)
            .flexShrink(0)
        }
        if (this.spec.count.length > 0) {
          Text(this.spec.count)
            .fontSize(WebType.text3)
            .lineHeight(WebType.text3Line)
            .fontColor($r('app.color.fg3'))
            .fontFeature('"tnum" on')
            .maxLines(1)
            .flexShrink(0)
        }
        if (this.spec.chevron.length > 0) {
          FoldMark({ open: this.spec.open, caret: this.spec.chevron === 'caret', tint: this.spec.chevron === 'caret' ? $r('app.color.action_fg') : $r('app.color.fg2') })
            .flexShrink(0)
        }
      }
      .alignItems(this.spec.wrap ? VerticalAlign.Top : VerticalAlign.Center)
      .layoutWeight(1)

      if (this.spec.add > 0 || this.spec.del > 0 || this.spec.meta.length > 0 || this.spec.outcome.length > 0 ||
        this.spec.outcomeSlot || this.spec.duration.length > 0 || this.spec.runSince > 0) {
        Row({ space: Metrics.s2 }) {
          if (this.spec.add > 0 || this.spec.del > 0) {
            DiffStatText({ add: this.spec.add, del: this.spec.del })
          }
          if (this.spec.meta.length > 0) {
            Text(this.spec.meta)
              .fontSize(WebType.text3)
              .lineHeight(WebType.text3Line)
              .fontColor(this.spec.metaInk ?? $r('app.color.fg3'))
              .fontFeature('"tnum" on')
              .maxLines(1)
          }
          if (this.spec.outcome.length > 0 || this.spec.outcomeSlot) {
            OutcomeMark({ outcome: this.spec.outcome })
          }
          if (this.spec.duration.length > 0) {
            Text(this.spec.duration)
              .fontSize(WebType.text3)
              .lineHeight(WebType.text3Line)
              .fontColor($r('app.color.fg3'))
              .fontFeature('"tnum" on')
              .maxLines(1)
          } else if (this.spec.runSince > 0) {
            LiveClock({ since: this.spec.runSince, now: this.spec.now, caption: true })
          }
        }
        .margin({ left: Metrics.gap })
        .flexShrink(0)
      }
    }
    .alignItems(this.spec.wrap ? VerticalAlign.Top : VerticalAlign.Center)
    .constraintSize({ minHeight: this.spec.height })
    .width('100%')
  }
}

/** DiffStat: only the sides that changed, +add in ok, −del in danger toned down to the green's weight */
@ComponentV2
export struct DiffStatText {
  @Param add: number = 0;
  @Param del: number = 0;

  build() {
    Row({ space: Metrics.s1 }) {
      if (this.add > 0) {
        Text(`+${this.add}`)
          .fontSize(WebType.text3)
          .lineHeight(WebType.text3Line)
          .fontColor($r('app.color.ok'))
          .fontFeature('"tnum" on')
      }
      if (this.del > 0) {
        Text(`−${this.del}`)
          .fontSize(WebType.text3)
          .lineHeight(WebType.text3Line)
          .fontColor($r('app.color.danger'))
          .opacity(0.8)
          .fontFeature('"tnum" on')
      }
    }
  }
}

/** A row the reader taps: the pressed fill reaches --row-hit-outset past the text column */
@ComponentV2
export struct TapRow {
  @Param spec: RowSpec = new RowSpec();
  @Event onTap: () => void = () => {};

  build() {
    Stack() {
      AcpRow({ spec: this.spec })
    }
    .width('100%')
    .borderRadius(Metrics.rMd)
    .stateStyles({
      pressed: {
        .backgroundColor($r('app.color.hover'))
      },
      normal: {
        .backgroundColor(Color.Transparent)
      }
    })
    .onClick(() => this.onTap())
  }
}

/**
 * The rail beside an open body (ConnectedRail): a line down from under the row's lead icon, ending on a solid dot. In
 * `rows` mode the body's rows sit at the head's own column (their icons hide the line behind them); otherwise the body
 * is indented to the label column
 */
@ComponentV2
export struct RailBody {
  @Param rows: boolean = false;
  @Param action: boolean = true;
  @Param top: number = Metrics.s1;
  @BuilderParam content: () => void;
  /** The body's measured height: the line runs its whole length */
  @Local h: number = 0;

  ink(): ResourceColor {
    return this.action ? $r('app.color.rail_action') : $r('app.color.rail_color');
  }

  build() {
    Stack({ alignContent: Alignment.TopStart }) {
      if (this.h > 0) {
        // Under the rows, so a row's lead square hides it where an icon stands; its top fades in over --rail-fade
        Column()
          .width(WebMetrics.railWidth)
          .height(this.h)
          .linearGradient({
            angle: 180,
            colors: [[Color.Transparent, 0], [this.ink(), Math.min(0.4, WebMetrics.railFade / this.h)], [this.ink(), 1]]
          })
          .position({ x: WebMetrics.icon / 2 - WebMetrics.railWidth / 2, y: 0 })
        Circle()
          .width(WebMetrics.railDot)
          .height(WebMetrics.railDot)
          .fill(this.ink())
          .position({ x: WebMetrics.icon / 2 - WebMetrics.railDot / 2, y: this.h - WebMetrics.railDot / 2 })
      }
      Column() {
        this.content()
      }
      .alignItems(HorizontalAlign.Start)
      .padding({ top: this.top, left: this.rows ? 0 : WebMetrics.indent })
      .width('100%')
      .onAreaChange((before: Area, after: Area) => {
        this.h = Number(after.height);
      })
    }
    .width('100%')
  }
}

/**
 * A running time in whole seconds on the shared tick (LiveElapsed / CommandSeconds). A fold head's clock stays hidden
 * for its first second, so a turn that ends at once shows no counter; a caption (a command's) shows from zero
 */
@ComponentV2
export struct LiveClock {
  @Param since: number = 0;
  @Param now: () => number = () => Date.now();
  @Param caption: boolean = false;

  aboutToAppear(): void {
    ticker.start();
  }

  text(): string {
    // Reads the tick, so the text follows the shared clock
    const ms = ticker.tick >= 0 ? this.now() - this.since : 0;
    return ms < 1000 && !this.caption ? '' : liveSeconds(ms);
  }

  build() {
    Text(this.text())
      .fontSize(this.caption ? WebType.text3 : WebType.text2)
      .lineHeight(this.caption ? WebType.text3Line : WebType.text2Line)
      .fontColor($r('app.color.fg3'))
      .fontFeature('"tnum" on')
      .maxLines(1)
  }
}

/** The live clock's text: whole seconds, then minutes and seconds (commandDuration `whole`) */
function liveSeconds(ms: number): string {
  const seconds = Math.floor(Math.max(0, ms) / 1000);
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  const params = new Map<string, string>();
  params.set('m', `${m}`);
  params.set('s', `${s}`);
  if (m > 0) {
    return s > 0 ? t('turns.elapsed.ms', params) : t('turns.elapsed.m', params);
  }
  return t('turns.elapsed.s', params);
}

/**
 * ui/Disclosure: the head row toggling its body, the body on the rail (`body`: indented to the label column; `rows`: its
 * rows at the head's column with the line through their icons; `off`: indented, no rail). The open state lives in
 * `folds` under `foldKey`, so a rebuilt row keeps the reader's choice
 */
@ComponentV2
export struct Disclosure {
  @Param spec: RowSpec = new RowSpec();
  @Param foldKey: string = '';
  @Param defaultOpen: boolean = false;
  @Param rail: string = 'body';
  @Param bodyTop: number = Metrics.s1;
  /** Room under the open body (a code card's gap to the next row) */
  @Param after: number = 0;
  @BuilderParam body: () => void;

  build() {
    Column() {
      TapRow({
        spec: this.spec.folded(folds.isOpen(this.foldKey, this.defaultOpen)),
        onTap: () => folds.toggle(this.getUIContext(), this.foldKey, this.defaultOpen)
      })
      if (folds.isOpen(this.foldKey, this.defaultOpen)) {
        Column() {
          if (this.rail === 'off') {
            Column() {
              this.body()
            }
            .alignItems(HorizontalAlign.Start)
            .padding({ top: this.bodyTop, left: WebMetrics.indent })
            .width('100%')
          } else {
            RailBody({ rows: this.rail === 'rows', action: this.spec.action, top: this.bodyTop }) {
              this.body()
            }
          }
        }
        .padding({ bottom: this.after })
        .width('100%')
        .transition(TransitionEffect.OPACITY.animation({ duration: WebMetrics.foldMs, curve: Curve.Smooth }))
      }
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
  }
}
