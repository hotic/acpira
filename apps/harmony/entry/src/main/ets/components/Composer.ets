import { Metrics, Type } from '../design/Tokens';
import { t } from '../store/L10n';
import { Lucide } from './Glyphs';

/**
 * The composer card (apps/ios Chat/Composer.swift): the field grows to its line budget then scrolls; the round button
 * sends, or stops the running turn while the field is empty
 */
@ComponentV2
export struct Composer {
  @Param placeholder: string = '';
  @Param running: boolean = false;
  @Event onSend: (text: string) => void = () => {};
  @Event onStop: () => void = () => {};
  @Local text: string = '';

  build() {
    Row({ space: Metrics.s2 }) {
      Lucide({ name: 'plus', tint: $r('app.color.fg1'), extent: Metrics.iconCtl })
        .margin({ left: Metrics.gap, bottom: Metrics.s2 + Metrics.s1 })
      TextArea({ placeholder: this.placeholder.length > 0 ? this.placeholder : t('composer.placeholder'), text: this.text })
        .id('composer.field')
        .fontSize(Type.text1)
        .fontColor($r('app.color.fg1'))
        .placeholderColor($r('app.color.fg3'))
        .placeholderFont({ size: Type.text1 })
        .backgroundColor(Color.Transparent)
        .maxLines(Metrics.composerMaxLines)
        .padding({ top: Metrics.s2 + Metrics.s1, bottom: Metrics.s2 + Metrics.s1, left: Metrics.s1, right: Metrics.s1 })
        .layoutWeight(1)
        .onChange((value: string) => {
          this.text = value;
        })
      Stack() {
        if (this.running && this.text.trim().length === 0) {
          Lucide({ name: 'square', tint: $r('app.color.btn1_fg'), extent: Metrics.icon - Metrics.s1 })
        } else {
          Lucide({
            name: 'arrowUp',
            tint: this.text.trim().length > 0 ? $r('app.color.btn1_fg') : $r('app.color.send_ink'),
            extent: Metrics.iconCtl
          })
        }
      }
      .id('composer.send')
      .width(Metrics.send)
      .height(Metrics.send)
      .borderRadius(Metrics.send / 2)
      .backgroundColor(this.text.trim().length > 0 || this.running ? $r('app.color.btn1_bg') : $r('app.color.send_bg'))
      .margin({ right: Metrics.s2, bottom: Metrics.s1 + Metrics.s1 / 2 })
      .onClick(() => {
        if (this.text.trim().length > 0) {
          this.onSend(this.text);
          this.text = '';
        } else if (this.running) {
          this.onStop();
        }
      })
    }
    .alignItems(VerticalAlign.Bottom)
    .backgroundColor($r('app.color.bg1'))
    .borderRadius(Metrics.rComposer)
    .border({ width: Metrics.hairline, color: $r('app.color.line') })
    .shadow({ radius: Metrics.gap, color: $r('app.color.line'), offsetY: Metrics.s1 })
    .constraintSize({ minHeight: Metrics.send + Metrics.gap, maxWidth: Metrics.contentW })
    .width('100%')
  }
}
