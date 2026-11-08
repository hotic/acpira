import { Metrics, PairMetrics, PairType, Type } from '../design/Tokens';
import { t } from '../store/L10n';
import { pathLabel } from '../store/PairingModel';
import { Lucide } from './Glyphs';

// The pairing screens' one centre piece (apps/ios Pairing/PairHero.swift, apps/android ui/Pairing.kt PairHero), laid
// out as an empty state is: a computer's glyph, its name, one sentence and one action. The empty list, connecting,
// connected and every failure use it, so moving between them swaps contents in a frame that stays: the glyph keeps its
// place, the sentence and the action keep their slots. Connected, the sentence gives way to the path the wire travels.

/** The one primary action: a solid capsule, black in light and white in dark, like the composer's send */
@ComponentV2
export struct PrimaryPill {
  @Param label: string = '';
  @Param on: boolean = true;
  @Event onTap: () => void = () => {};

  build() {
    Button(this.label)
      .type(ButtonType.Capsule)
      .fontSize(Type.body)
      .fontWeight(FontWeight.Medium)
      .fontColor($r('app.color.btn1_fg'))
      .backgroundColor($r('app.color.btn1_bg'))
      .height(Metrics.barCtl)
      .padding({ left: Metrics.page + Metrics.s2, right: Metrics.page + Metrics.s2 })
      .enabled(this.on)
      .opacity(this.on ? 1 : PairMetrics.disabled)
      .onClick(() => this.onTap())
  }
}

/** The path a paired computer's wire travels: its name on the active fill (ui/Button ChipTag) */
@ComponentV2
export struct PairPathTag {
  @Param path: string = 'lan';

  build() {
    Text(pathLabel(this.path))
      .id('pair.path')
      .fontSize(Type.text4)
      .fontColor($r('app.color.fg2'))
      .maxLines(1)
      .height(PairMetrics.pathH)
      .padding({ left: PairMetrics.pathX, right: PairMetrics.pathX })
      .borderRadius(PairMetrics.rXs)
      .backgroundColor($r('app.color.active'))
  }
}

/** The computer's glyph; paired, a check on its corner */
@ComponentV2
struct PairGlyph {
  @Param icon: string = 'laptop';
  @Param ink: ResourceColor = $r('app.color.fg2');
  @Param checked: boolean = false;

  build() {
    Stack({ alignContent: Alignment.BottomEnd }) {
      Lucide({ name: this.icon, tint: this.ink, extent: PairMetrics.glyph })
      if (this.checked) {
        Stack() {
          Lucide({ name: 'check', tint: $r('app.color.pair_check_ink'), extent: PairMetrics.checkIcon })
        }
        .width(PairMetrics.check)
        .height(PairMetrics.check)
        .borderRadius(PairMetrics.check / 2)
        .backgroundColor($r('app.color.ok'))
        .border({ width: PairMetrics.checkRing, color: $r('app.color.form_page') })
        .translate({ x: PairMetrics.checkX, y: PairMetrics.checkY })
        .transition(TransitionEffect.scale({ x: PairMetrics.checkFrom, y: PairMetrics.checkFrom }).combine(TransitionEffect.OPACITY))
      }
    }
    .width(PairMetrics.glyph)
    .height(PairMetrics.glyph)
  }
}

@ComponentV2
export struct PairHero {
  @Param icon: string = 'laptop';
  @Param line: string = '';
  /** The system spinner leads the sentence */
  @Param spin: boolean = false;
  @Param name: string | undefined = undefined;
  /** Paired: the glyph takes a check and the name says so */
  @Param paired: boolean = false;
  /** Paired and the wire up: the path it travels, in the sentence's place */
  @Param path: string | undefined = undefined;
  @Param action: string | undefined = undefined;
  /** A quieter way on under the action (scan another code after a failure) */
  @Param secondary: string | undefined = undefined;
  /** The computer does not answer: its glyph steps back */
  @Param dim: boolean = false;
  @Event onAction: () => void = () => {};
  @Event onSecondary: () => void = () => {};

  build() {
    Column() {
      Stack() {
        PairGlyph({
          icon: this.icon,
          ink: this.paired ? $r('app.color.fg1') : this.dim ? $r('app.color.fg3') : $r('app.color.fg2'),
          checked: this.paired
        })
      }
      .height(PairMetrics.glyphBox)
      .margin({ bottom: Metrics.page })

      if (this.name !== undefined) {
        Text(this.paired ? t('devices.paired', new Map<string, string>([['name', this.name]])) : this.name)
          .id('pair.name')
          .fontSize(PairType.nameSize)
          .lineHeight(PairType.nameLine)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg_strong'))
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
          .margin({ bottom: Metrics.s2 })
      }

      // The sentence, or the path's tag in its place; the slot keeps two lines so a longer one moves nothing under it
      Stack({ alignContent: Alignment.Top }) {
        if (this.path !== undefined) {
          PairPathTag({ path: this.path })
            .transition(TransitionEffect.OPACITY.animation({ duration: PairMetrics.swapMs }))
        } else {
          Row({ space: Metrics.s2 }) {
            if (this.spin) {
              LoadingProgress().color($r('app.color.fg2')).width(Metrics.iconCtl).height(Metrics.iconCtl)
            }
            Text(this.line)
              .id('pair.line')
              .fontSize(PairType.line)
              .lineHeight(PairType.lineLine)
              .fontColor($r('app.color.fg2'))
              .textAlign(TextAlign.Center)
              .flexShrink(1)
          }
          .transition(TransitionEffect.OPACITY.animation({ duration: PairMetrics.swapMs }))
        }
      }
      .width('100%')
      .constraintSize({ minHeight: PairMetrics.lineSlot })

      // The action's slot stays when there is none, so a phase without one moves nothing; it sits as far below the
      // sentence as the glyph sits above it
      Stack() {
        if (this.action !== undefined) {
          PrimaryPill({ label: this.action, onTap: () => this.onAction() }).id('pair.action')
        }
      }
      .margin({ top: Metrics.page })
      .constraintSize({ minHeight: Metrics.barCtl })

      if (this.secondary !== undefined) {
        Button(this.secondary)
          .id('pair.secondary')
          .type(ButtonType.Capsule)
          .fontSize(Type.body)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg2'))
          .backgroundColor(Color.Transparent)
          .height(Metrics.barCtl)
          .margin({ top: Metrics.s1 })
          .onClick(() => this.onSecondary())
      }
    }
    .padding({ left: Metrics.page, right: Metrics.page })
    .width('100%')
  }
}

/**
 * Puts the hero's glyph centre at the page's optical centre with everything else hanging under it, so a phase that
 * adds or drops a line keeps the glyph where it was
 */
@ComponentV2
export struct PairHeroPlacement {
  @BuilderParam content: () => void;
  @Local top: number = 0;

  build() {
    Scroll() {
      Column() {
        this.content()
      }
      .padding({ top: this.top })
      .width('100%')
    }
    .scrollBar(BarState.Off)
    .edgeEffect(EdgeEffect.None)
    .align(Alignment.Top)
    .width('100%')
    .height('100%')
    .onAreaChange((_old: Area, now: Area) => {
      const h = Number(now.height);
      this.top = Math.max(0, h * PairMetrics.heroRise - PairMetrics.glyphBox / 2);
    })
  }
}
