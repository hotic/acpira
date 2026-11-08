import { curves } from '@kit.ArkUI';
import { common } from '@kit.AbilityKit';
import { notificationManager } from '@kit.NotificationKit';
import { Metrics, Type, WelcomeMetrics } from '../design/Tokens';
import { AppModel } from '../store/AppModel';
import { t } from '../store/L10n';
import { PrefKey, prefs } from '../store/Prefs';
import { AgentMark, Lucide } from '../components/Glyphs';
import { GlassButton } from '../components/Chrome';
import { PairSheet } from './PairSheet';

// The first run (apps/ios Welcome/WelcomeFlow.swift, apps/android ui/Welcome.kt), over every screen while the app
// follows no engine and the flow never finished: the intro (the mark, the headline written in, the agents it works with,
// Start), then pairing a computer in place (pages/PairSheet.ets inline), with an engine by address and token as its
// fallback and the only step in a build without the remote access library, then notifications. Not now leaves the demo
// showing.

enum Step {
  Intro,
  Pair,
  Engine,
  Notify
}

/** The vendors the intro names, as the desktop's welcome lists the agents it found */
const VENDORS: string[] = ['claude', 'codex', 'gemini', 'grok', 'devin', 'kimi', 'opencode'];
/** Launch work lands in the first frames: the entrances start once they are past */
const START_DELAY_MS: number = 350;
/** Each vendor mark slides in a step after the one before */
const VENDOR_STEP_MS: number = 60;

/** The first run is due: no engine followed and the flow never finished */
export function welcomeDue(): boolean {
  return !prefs.bool(PrefKey.welcomed, false);
}

@ComponentV2
export struct WelcomeFlow {
  @Require @Param model: AppModel;
  /** The step to open on (screenshots): intro, pair, engine or notify */
  @Param start: string = 'intro';
  @Event onDone: () => void = () => {};
  @Local step: Step = Step.Intro;
  /** The intro's copy has come in */
  @Local shown: boolean = false;
  @Local address: string = '';
  @Local token: string = '';

  aboutToAppear(): void {
    this.step = this.start === 'engine' ? Step.Engine : this.start === 'notify' ? Step.Notify :
      this.start === 'pair' ? this.connectStep() : Step.Intro;
    if (this.start !== 'intro' || !this.model.motion) {
      this.shown = true;
      return;
    }
    setTimeout(() => {
      this.shown = true;
    }, START_DELAY_MS);
  }

  finish(): void {
    prefs.set(PrefKey.welcomed, true);
    this.onDone();
  }

  /** Notifications next while the system has not been asked, else the screens */
  connected(): void {
    let enabled = false;
    try {
      enabled = notificationManager.isNotificationEnabledSync();
    } catch (e) {
      enabled = false;
    }
    if (enabled || prefs.bool(PrefKey.notifyAsked, false)) {
      this.finish();
    } else {
      this.step = Step.Notify;
    }
  }

  /** Pairing when the build carries it, else the engine by address */
  connectStep(): Step {
    return this.model.pairing !== undefined ? Step.Pair : Step.Engine;
  }

  connect(): void {
    const a = this.address.trim();
    if (a.split(':').length !== 2 || this.token.trim().length === 0) {
      return;
    }
    this.model.follow(a, this.token.trim(), true);
    this.connected();
  }

  swap(): TransitionEffect {
    return TransitionEffect.OPACITY.combine(TransitionEffect.translate({ y: Metrics.s3 }))
      .animation({ duration: this.model.motion ? WelcomeMetrics.swapMs : 0, curve: Curve.EaseInOut });
  }

  build() {
    Stack() {
      if (this.step === Step.Intro) {
        this.intro()
      } else if (this.step === Step.Pair && this.model.pairing !== undefined) {
        PairSheet({
          pairing: this.model.pairing,
          inline: true,
          onClose: () => this.connected(),
          onBack: () => {
            this.step = Step.Intro;
          },
          onAddress: () => {
            this.step = Step.Engine;
          },
          onLater: () => this.finish()
        })
          .transition(this.swap())
      } else if (this.step === Step.Engine) {
        this.engine()
      } else {
        this.notify()
      }
    }
    .id('welcome')
    .backgroundColor($r('app.color.bg0'))
    .width('100%')
    .height('100%')
  }

  /** Entrance: up from a little low, `at` ms after the copy is due */
  rise(at: number): AnimateParam {
    return { duration: this.model.motion ? WelcomeMetrics.easeMs : 0, delay: this.model.motion ? at : 0, curve: curves.cubicBezierCurve(0.22, 0.8, 0.24, 1) };
  }

  @Builder
  intro() {
    Column() {
      Blank()
      // The mark, lit up solid
      AgentMark({ agent: 'acpira', name: 'Acpira', tint: $r('app.color.fg1'), extent: WelcomeMetrics.markMax })
        .opacity(this.shown ? 1 : 0)
        .scale({ x: this.shown ? 1 : 0.92, y: this.shown ? 1 : 0.92 })
        .animation(this.rise(0))
      Blank()
      Column() {
        Row({ space: Metrics.s2 }) {
          AgentMark({ agent: 'acpira', name: 'Acpira', tint: $r('app.color.fg1'), extent: WelcomeMetrics.brandMark })
          Text('Acpira').fontSize(Type.text2).fontWeight(FontWeight.Medium).fontColor($r('app.color.fg2'))
        }
        .opacity(this.shown ? 1 : 0)
        .translate({ y: this.shown ? 0 : WelcomeMetrics.rise })
        .animation(this.rise(WelcomeMetrics.brandAt))
        Text(t('mobile.welcome.hero.1'))
          .fontSize(WelcomeMetrics.headline)
          .lineHeight(WelcomeMetrics.headlineLine)
          .fontWeight(FontWeight.Bold)
          .fontColor($r('app.color.fg3'))
          .margin({ top: WelcomeMetrics.headlineGap })
          .opacity(this.shown ? 1 : 0)
          .translate({ y: this.shown ? 0 : WelcomeMetrics.rise })
          .animation(this.rise(WelcomeMetrics.hero1At))
        Text(t('mobile.welcome.hero.2'))
          .fontSize(WelcomeMetrics.headline)
          .lineHeight(WelcomeMetrics.headlineLine)
          .fontWeight(FontWeight.Bold)
          .fontColor($r('app.color.fg_strong'))
          .opacity(this.shown ? 1 : 0)
          .translate({ y: this.shown ? 0 : WelcomeMetrics.rise })
          .animation(this.rise(WelcomeMetrics.hero2At))
        Column({ space: WelcomeMetrics.agentsRow }) {
          Text(t('mobile.welcome.agents')).fontSize(Type.text3).fontColor($r('app.color.fg3'))
          Row({ space: WelcomeMetrics.agentStep }) {
            ForEach(VENDORS, (v: string, i: number) => {
              AgentMark({ agent: v, name: v, tint: $r('app.color.fg1'), extent: WelcomeMetrics.agentMark })
                .opacity(this.shown ? 1 : 0)
                .translate({ x: this.shown ? 0 : -Metrics.s1 })
                .animation(this.rise(WelcomeMetrics.factAt + VENDOR_STEP_MS * (i + 1)))
            }, (v: string) => v)
          }
        }
        .alignItems(HorizontalAlign.Start)
        .margin({ top: WelcomeMetrics.agentsGap })
        .opacity(this.shown ? 1 : 0)
        .animation(this.rise(WelcomeMetrics.factAt))
      }
      .alignItems(HorizontalAlign.Start)
      .padding({ left: WelcomeMetrics.copyInset })
      .width('100%')
      Button(t('welcome.start'))
        .id('welcome.start')
        .fontSize(Type.body)
        .fontWeight(FontWeight.Bold)
        .fontColor($r('app.color.btn1_fg'))
        .backgroundColor($r('app.color.btn1_bg'))
        .height(Metrics.barCtl)
        .width('100%')
        .margin({ top: WelcomeMetrics.actionsGap, bottom: Metrics.s2 })
        .opacity(this.shown ? 1 : 0)
        .translate({ y: this.shown ? 0 : WelcomeMetrics.rise })
        .animation(this.rise(WelcomeMetrics.actionsAt))
        .onClick(() => {
          this.step = this.connectStep();
        })
    }
    .constraintSize({ maxWidth: WelcomeMetrics.column })
    .padding({ left: Metrics.page, right: Metrics.page, top: Metrics.page, bottom: Metrics.page })
    .height('100%')
    .width('100%')
    .transition(this.swap())
  }

  @Builder
  field(text: string, placeholder: string, id: string, onChange: (v: string) => void) {
    TextInput({ text: text, placeholder: placeholder })
      .id(id)
      .fontSize(Type.text1)
      .fontColor($r('app.color.fg1'))
      .placeholderColor($r('app.color.fg3'))
      .backgroundColor($r('app.color.form_row'))
      .borderRadius(Metrics.barCtl / 2)
      .height(Metrics.barCtl)
      .padding({ left: Metrics.pad, right: Metrics.pad })
      .onChange(onChange)
      .onSubmit(() => this.connect())
  }

  /** The engine to follow: where it listens and its token, as `apps/ios/scripts/engine.sh` prints them */
  @Builder
  engine() {
    Column() {
      Row() {
        GlassButton({ icon: 'arrowLeft', tag: 'welcome.back', onTap: () => { this.step = this.model.pairing !== undefined ? Step.Pair : Step.Intro; } })
        Blank()
      }
      .height(Metrics.barCtl + Metrics.gap)
      .width('100%')
      Column({ space: Metrics.gap }) {
        Lucide({ name: 'laptop', tint: $r('app.color.fg1'), extent: WelcomeMetrics.bell })
        Text(t('mobile.pair.connectTitle'))
          .fontSize(WelcomeMetrics.notifyTitle)
          .fontWeight(FontWeight.Bold)
          .fontColor($r('app.color.fg_strong'))
          .margin({ top: Metrics.gap })
        Text(t('mobile.engine.desc'))
          .fontSize(Type.text2)
          .fontColor($r('app.color.fg2'))
          .textAlign(TextAlign.Center)
        Column({ space: Metrics.gap }) {
          this.field(this.address, 'host:port', 'welcome.address', (v: string) => {
            this.address = v;
          })
          this.field(this.token, 'token', 'welcome.token', (v: string) => {
            this.token = v;
          })
        }
        .margin({ top: Metrics.page })
      }
      .layoutWeight(1)
      .justifyContent(FlexAlign.Center)
      Column({ space: Metrics.s3 }) {
        Button(t('mobile.pair.connect'))
          .id('welcome.connect')
          .fontSize(Type.body)
          .fontWeight(FontWeight.Bold)
          .fontColor($r('app.color.btn1_fg'))
          .backgroundColor($r('app.color.btn1_bg'))
          .opacity(this.address.trim().length > 0 && this.token.trim().length > 0 ? 1 : 0.4)
          .height(Metrics.barCtl)
          .width('100%')
          .onClick(() => this.connect())
        Text(t('mobile.welcome.later'))
          .id('welcome.later')
          .fontSize(Type.text2)
          .fontColor($r('app.color.fg3'))
          .height(WelcomeMetrics.laterHeight)
          .onClick(() => this.finish())
      }
      .margin({ bottom: Metrics.s2 })
    }
    .constraintSize({ maxWidth: WelcomeMetrics.column })
    .padding({ left: Metrics.page, right: Metrics.page, bottom: Metrics.page })
    .height('100%')
    .width('100%')
    .transition(this.swap())
  }

  /** Notifications: the bell in its ring, what notices are for, then ask or later; either way the question is spent */
  @Builder
  notify() {
    Column() {
      Blank()
      Stack() {
        Circle()
          .width(WelcomeMetrics.bellRing)
          .height(WelcomeMetrics.bellRing)
          .fill(Color.Transparent)
          .stroke($r('app.color.line_strong'))
          .strokeWidth(Metrics.hairline)
        Lucide({ name: 'bell', tint: $r('app.color.fg1'), extent: WelcomeMetrics.bell })
      }
      Text(t('mobile.welcome.notify.title'))
        .fontSize(WelcomeMetrics.notifyTitle)
        .fontWeight(FontWeight.Bold)
        .fontColor($r('app.color.fg_strong'))
        .margin({ top: WelcomeMetrics.notifyTitleGap })
      Text(t('mobile.welcome.notify.desc'))
        .fontSize(Type.text1)
        .fontColor($r('app.color.fg2'))
        .textAlign(TextAlign.Center)
        .margin({ top: WelcomeMetrics.notifyDescGap })
      Blank()
      Column({ space: Metrics.s3 }) {
        Button(t('mobile.notify.ask'))
          .id('welcome.notify.ask')
          .fontSize(Type.body)
          .fontWeight(FontWeight.Bold)
          .fontColor($r('app.color.btn1_fg'))
          .backgroundColor($r('app.color.btn1_bg'))
          .height(Metrics.barCtl)
          .width('100%')
          .onClick(() => {
            prefs.set(PrefKey.notifyAsked, true);
            try {
              notificationManager.requestEnableNotification(this.getUIContext().getHostContext() as common.UIAbilityContext)
                .finally(() => this.finish());
            } catch (e) {
              this.finish();
            }
          })
        Text(t('mobile.welcome.later'))
          .id('welcome.notify.later')
          .fontSize(Type.text2)
          .fontColor($r('app.color.fg3'))
          .height(WelcomeMetrics.laterHeight)
          .onClick(() => {
            prefs.set(PrefKey.notifyAsked, true);
            this.finish();
          })
      }
      .margin({ bottom: Metrics.s2 })
    }
    .constraintSize({ maxWidth: WelcomeMetrics.column })
    .padding({ left: Metrics.page, right: Metrics.page, top: Metrics.page, bottom: Metrics.page })
    .height('100%')
    .width('100%')
    .transition(this.swap())
  }
}
