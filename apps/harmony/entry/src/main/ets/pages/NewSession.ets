import { Metrics, Type } from '../design/Tokens';
import { AppModel } from '../store/AppModel';
import { t } from '../store/L10n';
import { Lucide } from '../components/Glyphs';
import { Composer } from '../components/Composer';

// A new session (apps/ios Sessions/NewSessionView.swift): the prompt goes as its first turn, in the latest project.
// On a wide window it is the detail column's first page, beside the sidebar.

@ComponentV2
export struct NewSession {
  @Require @Param model: AppModel;
  @Param showBack: boolean = true;
  @Event onBack: () => void = () => {};
  @Event onStart: (text: string) => void = () => {};

  build() {
    Column() {
      Row({ space: Metrics.s2 }) {
        if (this.showBack) {
          Stack() {
            Lucide({ name: 'arrowLeft', tint: $r('app.color.fg1'), extent: Metrics.iconCtl })
          }
          .width(Metrics.ctl)
          .height(Metrics.ctl)
          .onClick(() => this.onBack())
        }
        Text(t('session.new'))
          .fontSize(Type.text1)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg1'))
          .layoutWeight(1)
      }
      .padding({ left: this.showBack ? Metrics.s1 : Metrics.pad, right: Metrics.s1 })
      .height(Metrics.barCtl + Metrics.s2)
      .width('100%')

      Column({ space: Metrics.gap }) {
        Text(this.model.summaries.length > 0 ? this.model.summaries[0].cwd.split('/').pop() ?? '' : '')
          .fontSize(Type.text3)
          .fontColor($r('app.color.fg3'))
      }
      .justifyContent(FlexAlign.Center)
      .layoutWeight(1)

      Row() {
        Composer({ placeholder: t('mobile.new.placeholder'), onSend: (text: string) => this.onStart(text) })
      }
      .justifyContent(FlexAlign.Center)
      .padding({ left: Metrics.gap, right: Metrics.gap, top: Metrics.s2, bottom: Metrics.s2 })
      .width('100%')
    }
    .backgroundColor($r('app.color.bg0'))
    .height('100%')
    .width('100%')
  }
}
