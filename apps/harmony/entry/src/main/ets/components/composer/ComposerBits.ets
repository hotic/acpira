import { PathShape } from '@kit.ArkUI';
import { Usage } from '../../wire/Wire';
import { LUCIDE } from '../../design/Icons';
import { Cmp, Metrics, Type } from '../../design/Tokens';
import { groupedNumber, UsageBreakdown, UsagePanel } from '../../logic/Composing';
import { t } from '../../store/L10n';
import { AgentMark, Lucide, scalePath } from '../Glyphs';

// The composer's small pieces (apps/ios Chat/Composer.swift SendButton, Chat/ContextUsage.swift, Design/Goo.swift
// FastGlyph / SidekickMark, Chat/Cards CardSurface / CardButton): shared by the composer, the beans and the dock.

/**
 * A filled lucide glyph in a resource colour: the colour fills a box clipped to the glyph's outline, since a Path's own
 * fill takes only a literal colour (a resource or a resolved one came out empty in the Previewer)
 */
@ComponentV2
export struct FilledGlyph {
  @Param name: string = 'zap';
  @Param tint: ResourceColor = $r('app.color.fg1');
  @Param extent: number = Metrics.icon;

  build() {
    Stack() {
      ForEach(LUCIDE.get(this.name) ?? [], (d: string) => {
        Column()
          .width(this.extent)
          .height(this.extent)
          .backgroundColor(this.tint)
          .clipShape(new PathShape({ commands: scalePath(d, this.getUIContext().vp2px(this.extent) / Metrics.iconBox) }))
          .position({ x: 0, y: 0 })
      }, (d: string, i: number) => `${i}`)
      Lucide({ name: this.name, tint: this.tint, extent: this.extent })
    }
    .width(this.extent)
    .height(this.extent)
  }
}

/** The Fast bolt: filled in --fast while Fast is on, a plain stroke otherwise */
@ComponentV2
export struct FastGlyph {
  @Param on: boolean = false;
  @Param extent: number = Cmp.chevron;

  build() {
    if (this.on) {
      FilledGlyph({ name: 'zap', tint: $r('app.color.fast'), extent: this.extent })
    } else {
      Lucide({ name: 'zap', tint: $r('app.color.fg1'), extent: this.extent })
    }
  }
}

/**
 * The sidekick's mark (Goo.swift SidekickMark, SidekickPair.tsx): its model's mark with the plus badge set into a ring
 * cut out of the mark's corner, or a faint fading plus while there is none
 */
@ComponentV2
export struct SidekickMark {
  /** The picked model's brand key and name; an empty name for none */
  @Param brand: string = '';
  @Param name: string = '';
  @Param extent: number = Cmp.barIcon;

  build() {
    if (this.name.length === 0) {
      Lucide({ name: 'circleFadingPlus', tint: $r('app.color.fg3'), extent: this.extent })
    } else {
      Stack({ alignContent: Alignment.BottomEnd }) {
        AgentMark({ agent: this.brand, name: this.name, tint: $r('app.color.fg1'), extent: this.extent })
        // The notch: a ring of the surface's colour round the badge
        Stack() {
          Lucide({ name: 'plus', tint: $r('app.color.menu_bg'), extent: Cmp.sidekickBadge - Cmp.sidekickNotch })
        }
        .width(Cmp.sidekickBadge)
        .height(Cmp.sidekickBadge)
        .borderRadius(Cmp.sidekickBadge / 2)
        .backgroundColor($r('app.color.fg1'))
        .outline({ width: Cmp.sidekickNotch, color: $r('app.color.menu_bg'), radius: Cmp.sidekickBadge / 2 + Cmp.sidekickNotch })
        .translate({ x: Cmp.sidekickNotch, y: Cmp.sidekickNotch })
      }
      .width(this.extent)
      .height(this.extent)
    }
  }
}

/**
 * Send / stop in one (SendButton): a solid circle set into the bar's end, lit (btn-1) once there is something to send or
 * a run to stop, a quiet chip fill otherwise. The bar-tall slot keeps the bar's layout
 */
@ComponentV2
export struct SendButton {
  @Param running: boolean = false;
  @Param filled: boolean = false;
  @Event action: () => void = () => {};

  build() {
    Stack() {
      Stack() {
        if (this.running && !this.filled) {
          Column()
            .width(Cmp.barIcon / 2)
            .height(Cmp.barIcon / 2)
            .borderRadius(Cmp.s0_5)
            .backgroundColor($r('app.color.btn1_fg'))
        } else {
          Lucide({
            name: 'arrowUp',
            tint: this.running || this.filled ? $r('app.color.btn1_fg') : $r('app.color.send_ink'),
            extent: Metrics.iconCtl
          })
        }
      }
      .width(Cmp.bar - Metrics.s1 * 2)
      .height(Cmp.bar - Metrics.s1 * 2)
      .borderRadius((Cmp.bar - Metrics.s1 * 2) / 2)
      .backgroundColor(this.running || this.filled ? $r('app.color.btn1_bg') : $r('app.color.chip'))
      .animation({ duration: Cmp.quickMs })
    }
    .id(this.running && !this.filled ? 'composer.stop' : 'composer.send')
    .width(Cmp.bar)
    .height(Cmp.bar)
    .enabled(this.running || this.filled)
    .accessibilityText(this.running && !this.filled ? t('composer.stop') : t('composer.send'))
    .onClick(() => this.action())
  }
}

/**
 * The context window ring beside send (ContextUsage.swift ContextRing): an icon-sized arc on a faint track in a slot as
 * tall as the bar; a tap opens its card with the share, the figures, the auto-compact threshold and Compact
 */
@ComponentV2
export struct ContextRing {
  @Require @Param usage: Usage;
  @Param canCompact: boolean = false;
  @Param compactAt: number | undefined = undefined;
  @Param running: boolean = false;
  /** Opened on appear (`popup: context`, screenshots) */
  @Param startOpen: boolean = false;
  @Event onCompact: () => void = () => {};
  @Local open: boolean = false;

  aboutToAppear(): void {
    this.open = this.startOpen;
  }

  build() {
    Stack() {
      Progress({ value: 1, total: 1, type: ProgressType.Ring })
        .style({ strokeWidth: Metrics.icon / Cmp.ringParts })
        .color($r('app.color.fg2'))
        .opacity(Cmp.ringTrack)
        .width(Metrics.icon - Metrics.icon / Cmp.ringParts * 2)
        .height(Metrics.icon - Metrics.icon / Cmp.ringParts * 2)
      Progress({ value: UsageBreakdown.fill(this.usage) * 100, total: 100, type: ProgressType.Ring })
        .style({ strokeWidth: Metrics.icon / Cmp.ringParts, shadow: false, status: ProgressStatus.PROGRESSING })
        .color($r('app.color.fg2'))
        .backgroundColor(Color.Transparent)
        .opacity(this.usage.compactedFrom !== undefined ? Cmp.ringStale : 1)
        .width(Metrics.icon - Metrics.icon / Cmp.ringParts * 2)
        .height(Metrics.icon - Metrics.icon / Cmp.ringParts * 2)
    }
    .id('composer.context')
    .width(Cmp.ctlSm)
    .height(Cmp.bar)
    .accessibilityText(this.usage.compactedFrom !== undefined ? t('usage.compactedStale') :
      t('usage.usedPct', new Map<string, string>([['pct', `${UsageBreakdown.percent(this.usage)}`]])))
    .onClick(() => {
      this.open = !this.open;
    })
    .bindPopup(this.open, {
      builder: () => {
        this.card();
      },
      placement: Placement.TopRight,
      popupColor: $r('app.color.menu_bg'),
      backgroundBlurStyle: BlurStyle.NONE,
      radius: Cmp.rGooPanel,
      enableArrow: false,
      onStateChange: (e) => {
        if (!e.isVisible) {
          this.open = false;
        }
      }
    })
  }

  private tickColor(tone: string): ResourceColor {
    switch (tone) {
      case 'ok':
        return $r('app.color.fg1');
      case 'pending':
        return $r('app.color.brand');
      case 'limited':
        return $r('app.color.warn');
      default:
        return $r('app.color.fg3');
    }
  }

  private inkColor(tone: string): ResourceColor {
    switch (tone) {
      case 'pending':
        return $r('app.color.brand');
      case 'limited':
        return $r('app.color.warn');
      default:
        return $r('app.color.fg3');
    }
  }

  /** UsagePanel: the title, the share and the figures, the bar with the threshold's tick, what that means, Compact */
  @Builder
  card() {
    Column({ space: Metrics.s2 }) {
      Text(t('usage.title'))
        .fontSize(Type.text2)
        .fontWeight(FontWeight.Medium)
        .fontColor($r('app.color.fg1'))
      Row({ space: Metrics.s2 }) {
        Text(this.usage.compactedFrom !== undefined ? t('usage.compactedStale') :
          t('usage.usedPctShort', new Map<string, string>([['pct', `${UsageBreakdown.percent(this.usage)}`]])))
          .fontSize(Type.text3)
          .fontColor($r('app.color.fg2'))
          .layoutWeight(1)
        Text(this.usage.compactedFrom !== undefined ?
          t('usage.beforeCompact', new Map<string, string>([['n', groupedNumber(this.usage.compactedFrom)]])) :
          `${groupedNumber(this.usage.used)} / ${groupedNumber(this.usage.size)}`)
          .fontSize(Type.text3)
          .fontColor($r('app.color.fg3'))
          .fontFeature('"tnum" on')
      }
      .width('100%')
      Row({ space: Metrics.s2 }) {
        Stack({ alignContent: Alignment.Start }) {
          Column().width('100%').height(Cmp.s1_5).backgroundColor($r('app.color.active'))
          Column()
            .width(`${UsageBreakdown.fill(this.usage) * 100}%`)
            .height(Cmp.s1_5)
            .borderRadius(Cmp.s1_5 / 2)
            .backgroundColor($r('app.color.fg2'))
            .opacity(this.usage.compactedFrom !== undefined ? Cmp.ringStale : 1)
          if (this.compactAt !== undefined && this.usage.size > 0) {
            Column()
              .width(Cmp.s0_5)
              .height(Cmp.s1_5)
              .backgroundColor(this.tickColor(this.panel().tone))
              .position({ x: `${Math.min(this.compactAt / this.usage.size, 0.99) * 100}%`, y: 0 })
          }
        }
        .height(Cmp.s1_5)
        .borderRadius(Cmp.s1_5 / 2)
        .clip(true)
        .layoutWeight(1)
        if (this.compactAt !== undefined) {
          Text(groupedNumber(this.compactAt))
            .fontSize(Type.text3)
            .fontFeature('"tnum" on')
            .fontColor(this.inkColor(this.panel().tone))
            .opacity(this.panel().tone === 'unsupported' ? Cmp.ringNoteFaint : 1)
        }
      }
      .width('100%')
      if (this.usage.compactedFrom !== undefined || this.panel().note !== undefined) {
        Text(this.usage.compactedFrom !== undefined ? t('usage.compactedStaleHint') : this.panel().note ?? '')
          .fontSize(Type.text3)
          .lineHeight(Cmp.text3Line)
          .fontColor($r('app.color.fg3'))
      }
      if (this.canCompact) {
        CardButton({ label: t('usage.compact'), icon: 'shrink', primary: false, enable: !this.running, onTap: () => {
          this.open = false;
          this.onCompact();
        } })
          .margin({ top: Metrics.s1 })
      }
    }
    .alignItems(HorizontalAlign.Start)
    .padding(Cmp.s3)
    .width(Cmp.popLg)
  }

  private panel(): UsagePanel {
    return UsageBreakdown.panel(this.usage, this.compactAt, this.canCompact, this.running);
  }
}

/**
 * A dock card's action (CardButton): a capsule; the way on is prominent in the strong ink, and falls back to the plain
 * fill with faint text while it cannot run. One line, never wrapped
 */
@ComponentV2
export struct CardButton {
  @Param label: string = '';
  @Param primary: boolean = false;
  @Param enable: boolean = true;
  /** A lucide glyph before the label */
  @Param icon: string = '';
  /** A chevron after the label (a menu) */
  @Param caret: boolean = false;
  @Event onTap: () => void = () => {};

  build() {
    Row({ space: Cmp.s1_5 }) {
      if (this.icon.length > 0) {
        Lucide({ name: this.icon, tint: this.lit() ? $r('app.color.bg0') : $r('app.color.fg1'), extent: Metrics.icon })
      }
      Text(this.label)
        .fontSize(Type.text2)
        .fontWeight(this.lit() ? FontWeight.Medium : FontWeight.Normal)
        .fontColor(this.lit() ? $r('app.color.bg0') : this.enable ? $r('app.color.fg1') : $r('app.color.pop_fg3'))
        .maxLines(1)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
      if (this.caret) {
        Lucide({ name: 'chevronDown', tint: $r('app.color.pop_fg3'), extent: Cmp.chevron })
      }
    }
    .height(Metrics.ctl)
    .padding({ left: Metrics.pad, right: Metrics.pad })
    .borderRadius(Metrics.ctl / 2)
    .backgroundColor(this.lit() ? $r('app.color.fg_strong') : $r('app.color.active'))
    .onClick(() => {
      if (this.enable) {
        this.onTap();
      }
    })
  }

  private lit(): boolean {
    return this.primary && this.enable;
  }
}

/** A dock card's corner control (close, retry): a small round button */
@ComponentV2
export struct CardCorner {
  @Param icon: string = 'x';
  @Param label: string = '';
  @Event onTap: () => void = () => {};

  build() {
    Stack() {
      Lucide({ name: this.icon, tint: $r('app.color.fg1'), extent: Metrics.icon })
    }
    .width(Cmp.ctlSm)
    .height(Cmp.ctlSm)
    .borderRadius(Cmp.ctlSm / 2)
    .backgroundColor($r('app.color.active'))
    .accessibilityText(this.label)
    .onClick(() => this.onTap())
  }
}

/** The dock cards' surface (CardSurface): the composer bar's own fill, corners, hairline and shadow */
export class DockCard implements AttributeModifier<ColumnAttribute> {
  applyNormalAttribute(instance: ColumnAttribute): void {
    instance
      .width('100%')
      .backgroundColor($r('app.color.bg1'))
      .borderRadius(Cmp.rCard)
      .border({ width: Metrics.hairline, color: $r('app.color.line') })
      .shadow({ radius: Cmp.shadow, color: $r('app.color.line'), offsetY: Cmp.s0_5 });
  }
}

export const DOCK_CARD: DockCard = new DockCard();
