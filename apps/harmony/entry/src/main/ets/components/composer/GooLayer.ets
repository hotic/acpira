import { curves } from '@kit.ArkUI';
import { Cmp, Metrics, Type } from '../../design/Tokens';
import { Goo } from '../../store/Composing';
import { AgentMark, Lucide } from '../Glyphs';
import { Bean, BeanGlyph, GooChoice } from './Beans';
import { FastGlyph, SidekickMark } from './ComposerBits';

// What the screen draws over an open picker (apps/ios Design/Goo.swift gooLayer, apps/android ui/Composer.kt GooLayer):
// the beans rising out of their chip with their values beside them, and a bean's panel resting on the column's
// bottom-left corner. Drawn over the whole screen, the composer included, so a tall card never hides a bean; the page
// under it blurs and dims (ChatPage). A tap on the open chip goes one level back, anywhere else closes.

function ink(tint: string): ResourceColor {
  switch (tint) {
    case 'fg3':
      return $r('app.color.fg3');
    case 'warn':
      return $r('app.color.warn');
    default:
      return $r('app.color.fg1');
  }
}

/** A bean's or a row's glyph */
@Builder
export function BeanGlyphView(g: BeanGlyph, extent: number) {
  if (g.kind === 'mark') {
    AgentMark({ agent: g.name, name: g.label, tint: ink(g.tint), extent: extent })
  } else if (g.kind === 'sidekick') {
    SidekickMark({ brand: g.name, name: g.label, extent: extent })
  } else if (g.kind === 'fast') {
    FastGlyph({ on: g.on, extent: extent })
  } else {
    Lucide({ name: g.name, tint: ink(g.tint), extent: extent })
  }
}

/** The panel's height: its rows up to gooPanelRows, no taller than the room above the chip */
function panelHeight(rows: number, room: number): number {
  const pad = Cmp.s1_5 * 2;
  const natural = Math.min(rows, Cmp.gooPanelRows) * Cmp.gooRow + pad;
  return Math.max(Cmp.gooRow + pad, Math.min(natural, room));
}

@ComponentV2
export struct GooLayer {
  @Require @Param goo: Goo;
  /** The open chip's beans */
  @Param beans: Bean[] = [];
  /** The layer's own place in the window, so the chips' window positions land in its frame */
  @Local originX: number = 0;
  @Local originY: number = 0;

  private left(): number {
    return (this.goo.last === 'attach' ? this.goo.attachLeft : this.goo.modelLeft) - this.originX;
  }

  /** The column's foot: a gap over the chips' top */
  private bottom(): number {
    return this.goo.chipTop - this.originY - Cmp.gooGap;
  }

  private spring(lag: number): AnimateParam {
    return { curve: curves.springMotion(Cmp.gooResponse, Cmp.gooDamping), delay: lag };
  }

  private tapBean(b: Bean, i: number): void {
    if (b.tap !== undefined) {
      b.tap();
    } else {
      this.goo.tap(i);
    }
  }

  build() {
    Stack() {
      if (this.goo.open !== undefined) {
        ForEach(this.beans, (b: Bean, i: number) => {
          this.bean(b, i)
        }, (b: Bean) => `${this.goo.open}.${b.key}`)
        ForEach(this.beans, (b: Bean, i: number) => {
          if (this.goo.stage === i + 1 && b.rows.length > 0) {
            this.panel(b)
          }
        }, (b: Bean) => `panel.${this.goo.open}.${b.key}`)
      }
    }
    .width('100%')
    .height('100%')
    .alignContent(Alignment.TopStart)
    .hitTestBehavior(this.goo.open !== undefined ? HitTestMode.Default : HitTestMode.None)
    .onAreaChange((_: Area, now: Area) => {
      this.originX = now.globalPosition.x as number;
      this.originY = now.globalPosition.y as number;
    })
    .onClick((e: ClickEvent) => {
      // On the open chip one level back (its × or ‹), on the other chip that one opens, anywhere else everything closes
      const onChip = (left: number): boolean => e.windowX >= left && e.windowX <= left + Cmp.bar &&
        e.windowY >= this.goo.chipTop && e.windowY <= this.goo.chipTop + Cmp.bar;
      const other = this.goo.open === 'attach' ? 'model' : 'attach';
      if (onChip(this.goo.open === 'attach' ? this.goo.attachLeft : this.goo.modelLeft)) {
        this.goo.back();
      } else if (other === 'attach' ? onChip(this.goo.attachLeft) : this.goo.modelChip && onChip(this.goo.modelLeft)) {
        this.goo.toggle(other);
      } else {
        this.goo.close();
      }
    })
  }

  /** One bean: its glass drop and the value beside it, gone into an open panel */
  @Builder
  bean(b: Bean, i: number) {
    Row({ space: Metrics.s2 }) {
      Stack() {
        BeanGlyphView(b.glyph, Cmp.barIcon)
      }
      .id(`goo.${b.key}`)
      .width(Cmp.bar)
      .height(Cmp.bar)
      .borderRadius(Cmp.bar / 2)
      .backgroundColor($r('app.color.menu_bg'))
      .border({ width: Metrics.hairline, color: $r('app.color.menu_line') })
      .accessibilityText(b.title)
      .onClick(() => this.tapBean(b, i))
      // The value beside it, over a soft glow of the page's colour so it reads over whatever lies behind
      Column({ space: Cmp.s0_5 }) {
        Text(b.caption)
          .fontSize(Type.text2)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg_strong'))
          .maxLines(1)
          .textShadow({ radius: Metrics.s2, color: $r('app.color.bg0') })
        if (b.note !== undefined) {
          Text(b.note)
            .fontSize(Type.text3)
            .fontColor($r('app.color.fg2'))
            .maxLines(2)
            .textShadow({ radius: Metrics.s2, color: $r('app.color.bg0') })
        }
      }
      .alignItems(HorizontalAlign.Start)
      .constraintSize({ maxWidth: Cmp.gooAside })
      .padding({ left: Metrics.s1, right: Metrics.s1 })
      .onClick(() => this.tapBean(b, i))
    }
    .position({ x: this.left(), y: this.bottom() - Cmp.bar - i * (Cmp.bar + Cmp.gooGap) })
    // Every bean goes into an open panel, its own too
    .opacity(this.goo.stage > 0 ? 0 : 1)
    .scale({ x: this.goo.stage > 0 ? Cmp.gooSeed : 1, y: this.goo.stage > 0 ? Cmp.gooSeed : 1, centerX: Cmp.bar / 2 })
    .enabled(this.goo.stage === 0)
    .animation({ curve: curves.springMotion(Cmp.gooResponse, Cmp.gooDamping) })
    // Squeezed out of the chip, each a moment after the one before
    .transition(TransitionEffect.translate({ y: (i + 1) * (Cmp.bar + Cmp.gooGap) })
      .combine(TransitionEffect.scale({ x: Cmp.gooSeed, y: Cmp.gooSeed, centerX: Cmp.bar / 2 }))
      .combine(TransitionEffect.OPACITY)
      .animation(this.spring(i * Cmp.gooLagMs)))
  }

  /** A bean's panel, grown out of the column's bottom-left corner: its rows in a scroll that opens on the current one */
  @Builder
  panel(b: Bean) {
    Column() {
      GooList({ rows: b.rows, current: b.current })
    }
    .width(Cmp.gooPanelW)
    .height(panelHeight(b.rows.length, this.bottom() - Cmp.bar * 2))
    .padding({ top: Cmp.s1_5, bottom: Cmp.s1_5 })
    .backgroundColor($r('app.color.menu_bg'))
    .border({ width: Metrics.hairline, color: $r('app.color.menu_line') })
    .borderRadius(Cmp.rGooPanel)
    .clip(true)
    .position({ x: this.left(), y: this.bottom() - panelHeight(b.rows.length, this.bottom() - Cmp.bar * 2) })
    // Taps inside the panel stay there
    .onClick(() => {})
    .transition(TransitionEffect.scale({
      x: Cmp.panelSeed, y: Cmp.panelSeed, centerX: 0,
      centerY: panelHeight(b.rows.length, this.bottom() - Cmp.bar * 2)
    }).combine(TransitionEffect.OPACITY).animation(this.spring(0)))
  }
}

/** A panel's rows, scrolled so the current one shows */
@ComponentV2
export struct GooList {
  @Param rows: GooChoice[] = [];
  @Param current: number = -1;
  private scroller: Scroller = new Scroller();

  build() {
    Scroll(this.scroller) {
      Column() {
        ForEach(this.rows, (row: GooChoice) => {
          GooRowView({ row: row })
        }, (row: GooChoice) => `${row.id}:${row.checked}:${row.toggle?.on ?? ''}`)
      }
      .width('100%')
    }
    .scrollBar(BarState.Off)
    .edgeEffect(EdgeEffect.None)
    .height('100%')
    .onAppear(() => {
      if (this.current > 1) {
        this.scroller.scrollTo({ xOffset: 0, yOffset: Cmp.gooRow * (this.current - 2) });
      }
    })
  }
}

/** One choice in a panel: glyph, name, and the check (or Thinking's switch) */
@ComponentV2
export struct GooRowView {
  @Require @Param row: GooChoice;

  build() {
    Row({ space: Metrics.s2 }) {
      if (this.row.glyph !== undefined) {
        BeanGlyphView(this.row.glyph, Metrics.iconCtl)
      }
      Text(this.row.name)
        .fontSize(Type.text2)
        .fontWeight(this.row.checked ? FontWeight.Medium : FontWeight.Normal)
        .fontColor(this.row.checked ? $r('app.color.fg_strong') : $r('app.color.pop_fg2'))
        .maxLines(1)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
        .layoutWeight(1)
      if (this.row.toggle !== undefined) {
        Toggle({ type: ToggleType.Switch, isOn: this.row.toggle.on })
          .selectedColor($r('app.color.ok'))
          .enabled(!this.row.toggle.disabled)
          .onChange((on: boolean) => this.row.toggle?.set(on))
      } else if (this.row.note !== undefined) {
        Text(this.row.note)
          .fontSize(Type.text3)
          .fontColor($r('app.color.fg3'))
          .maxLines(1)
      } else {
        Lucide({ name: 'check', tint: $r('app.color.fg_strong'), extent: Metrics.icon })
          .opacity(this.row.checked ? 1 : 0)
      }
    }
    .width('100%')
    .height(Cmp.gooRow)
    .padding({ left: Cmp.gooPadX, right: Cmp.gooPadX })
    .opacity(this.row.disabled ? Cmp.dimmed : 1)
    .enabled(!this.row.disabled)
    .stateStyles({
      pressed: {
        .backgroundColor($r('app.color.pop_hover'))
      },
      normal: {
        .backgroundColor(Color.Transparent)
      }
    })
    .onClick(() => this.row.action())
  }
}
