import { ListMetrics, Metrics, Type } from '../design/Tokens';
import { AppModel, Connection } from '../store/AppModel';
import { t } from '../store/L10n';
import { Lucide } from './Glyphs';

// The chrome every page shares (apps/ios Design/Glass*, Store/AppModel.swift ToastView): the bars' round glass
// controls, the toast dropping in under the bar, and the banner a page shows while the engine's wire is down.

/** A round glass control in a bar: the list's filter and settings, a page's back */
@ComponentV2
export struct GlassButton {
  @Param icon: string = 'square';
  @Param tag: string = '';
  @Event onTap: () => void = () => {};

  build() {
    Stack() {
      Lucide({ name: this.icon, tint: $r('app.color.fg1'), extent: Metrics.iconCtl })
    }
    .id(this.tag)
    .width(Metrics.barCtl)
    .height(Metrics.barCtl)
    .borderRadius(Metrics.barCtl / 2)
    .backgroundColor($r('app.color.glass_bg'))
    .border({ width: Metrics.hairline, color: $r('app.color.glass_line') })
    .shadow({ radius: ListMetrics.barShadow, color: $r('app.color.line'), offsetY: Metrics.s1 / 2 })
    .onClick(() => this.onTap())
  }
}

/** Two or more glass controls sharing one capsule (the chat bar's new session and "…") */
@ComponentV2
export struct GlassCapsule {
  @BuilderParam content: () => void;

  build() {
    Row() {
      this.content()
    }
    .height(Metrics.barCtl)
    .borderRadius(Metrics.barCtl / 2)
    .backgroundColor($r('app.color.glass_bg'))
    .border({ width: Metrics.hairline, color: $r('app.color.glass_line') })
    .shadow({ radius: ListMetrics.barShadow, color: $r('app.color.line'), offsetY: Metrics.s1 / 2 })
  }
}

/** One control inside a GlassCapsule */
@ComponentV2
export struct CapsuleItem {
  @Param icon: string = 'square';
  @Param tag: string = '';
  @Event onTap: () => void = () => {};

  build() {
    Stack() {
      Lucide({ name: this.icon, tint: $r('app.color.fg1'), extent: Metrics.iconCtl })
    }
    .id(this.tag)
    .width(Metrics.barCtl + Metrics.s2)
    .height(Metrics.barCtl)
    .onClick(() => this.onTap())
  }
}

/**
 * The model's toast over every page, dropping in from the top and leaving after a while (one with an undo stays
 * longer, and only it takes taps)
 */
@ComponentV2
export struct ToastLayer {
  @Require @Param model: AppModel;
  private seq: number = 0;

  @Monitor('model.toast')
  onToast(): void {
    const text = this.model.toast;
    if (text === undefined) {
      return;
    }
    const mine = ++this.seq;
    const undo = this.model.lists.toastAction?.toast === text;
    setTimeout(() => {
      if (this.seq === mine && this.model.toast === text) {
        this.model.toast = undefined;
      }
    }, undo ? ListMetrics.toastUndoMs : ListMetrics.toastMs);
  }

  build() {
    Column() {
      if (this.model.toast !== undefined) {
        Row({ space: Metrics.s2 }) {
          Text(this.model.toast)
            .fontSize(Type.text2)
            .fontWeight(FontWeight.Medium)
            .fontColor($r('app.color.fg_strong'))
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
            .flexShrink(1)
          if (this.model.lists.toastAction !== undefined && this.model.lists.toastAction?.toast === this.model.toast) {
            // The desktop toast's secondary button, as the capsule's last word after a hairline
            Divider()
              .vertical(true)
              .height(Metrics.iconCtl)
              .color($r('app.color.line_strong'))
            Text(this.model.lists.toastAction?.label ?? '')
              .id('toast.action')
              .fontSize(Type.text2)
              .fontWeight(FontWeight.Bold)
              .fontColor($r('app.color.fg_strong'))
              .height('100%')
              .onClick(() => this.model.lists.runToastAction())
          }
        }
        .id('toast')
        .padding({ left: Metrics.pad, right: Metrics.pad })
        .height(ListMetrics.toast)
        .borderRadius(ListMetrics.toast / 2)
        .backgroundColor($r('app.color.glass_bg'))
        .border({ width: Metrics.hairline, color: $r('app.color.glass_line') })
        .shadow({ radius: ListMetrics.toastShadow, color: $r('app.color.scrim'), offsetY: Metrics.s1 })
        .margin({ left: Metrics.page, right: Metrics.page })
        .transition(TransitionEffect.OPACITY.combine(TransitionEffect.move(TransitionEdge.TOP))
          .animation({ duration: this.model.motion ? ListMetrics.motionMs : 0 }))
      }
    }
    // Only as wide as the capsule, and clear of the bar by a margin (outside the hit area): an undo toast blocks touches
    // on itself alone, never on the bar's buttons
    .constraintSize({ maxWidth: '100%' })
    .margin({ top: Metrics.barCtl + Metrics.s2 })
    .alignItems(HorizontalAlign.Center)
    .hitTestBehavior(toastHitTest(this.model))
  }
}

/** An undo action takes taps on the capsule; a plain toast lets every touch through. The call site sets it too */
export function toastHitTest(model: AppModel): HitTestMode {
  return model.toast !== undefined && model.lists.toastAction !== undefined ? HitTestMode.Default : HitTestMode.Transparent;
}

/**
 * The engine's wire state where a page needs it said (the list's empty state, the sidebar's foot): connecting, or
 * the wire lost; nothing while it is up or the demo shows
 */
export function connectionNote(model: AppModel): string | undefined {
  if (model.connection === Connection.Connecting) {
    return t('connections.state.connecting');
  }
  if (model.connection === Connection.Closed) {
    return t('mobile.engine.disconnected');
  }
  return undefined;
}

/** A banner under a page's bar while the wire is down: the state, and a tap reconnects */
@ComponentV2
export struct ConnectionBanner {
  @Require @Param model: AppModel;
  @Event onRetry: () => void = () => {};

  build() {
    if (this.model.connection === Connection.Closed || this.model.connection === Connection.Connecting) {
      Row({ space: Metrics.s2 }) {
        if (this.model.connection === Connection.Connecting) {
          LoadingProgress().color($r('app.color.fg3')).width(Metrics.icon).height(Metrics.icon)
        } else {
          Circle().width(ListMetrics.footDot).height(ListMetrics.footDot).fill($r('app.color.warn'))
        }
        Text(connectionNote(this.model) ?? '')
          .fontSize(Type.text3)
          .fontColor($r('app.color.fg2'))
          .maxLines(1)
          .layoutWeight(1)
        if (this.model.connection === Connection.Closed) {
          Lucide({ name: 'refreshCw', tint: $r('app.color.fg2'), extent: Metrics.icon })
        }
      }
      .id('connection.banner')
      .padding({ left: Metrics.pad, right: Metrics.pad })
      .height(Metrics.ctl)
      .margin({ left: Metrics.pad, right: Metrics.pad, bottom: Metrics.s2 })
      .borderRadius(Metrics.ctl / 2)
      .backgroundColor($r('app.color.bg2'))
      .onClick(() => {
        if (this.model.connection === Connection.Closed) {
          this.onRetry();
        }
      })
    }
  }
}
