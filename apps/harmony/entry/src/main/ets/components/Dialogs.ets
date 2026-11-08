import { ComponentContent } from '@kit.ArkUI';
import { FormMetrics, Metrics, Type } from '../design/Tokens';
import { t } from '../store/L10n';

// The alerts the list, the chat and the settings raise (apps/ios's rename / delete / path alerts, apps/android
// RenameDialog / DeleteConfirm): a text typed anew in a card over a dimmed page, or a destructive action confirmed.

/** What a text prompt asks for, and where its answer goes */
export class PromptArgs {
  title: string = '';
  initial: string = '';
  placeholder: string = '';
  /** A second field (the engine's token), shown when its placeholder is set */
  secondPlaceholder: string = '';
  secondInitial: string = '';
  hint: string = '';
  save: string = '';
  onSave: (text: string, second: string) => void = () => {};
  close: () => void = () => {};
}

@ComponentV2
struct TextPrompt {
  @Param args: PromptArgs = new PromptArgs();
  @Local text: string = '';
  @Local second: string = '';

  aboutToAppear(): void {
    this.text = this.args.initial;
    this.second = this.args.secondInitial;
  }

  submit(): void {
    if (this.text.trim().length === 0) {
      return;
    }
    this.args.close();
    this.args.onSave(this.text, this.second);
  }

  @Builder
  field(text: string, placeholder: string, id: string, onChange: (v: string) => void) {
    TextInput({ text: text, placeholder: placeholder })
      .id(id)
      .fontSize(Type.text1)
      .fontColor($r('app.color.fg1'))
      .placeholderColor($r('app.color.fg3'))
      .backgroundColor($r('app.color.bg2'))
      .borderRadius(Metrics.rMd)
      .height(Metrics.ctl)
      .onChange(onChange)
      .onSubmit(() => this.submit())
  }

  build() {
    Column({ space: Metrics.gap }) {
      Text(this.args.title)
        .fontSize(Type.text1)
        .fontWeight(FontWeight.Medium)
        .fontColor($r('app.color.fg_strong'))
        .textAlign(TextAlign.Center)
        .width('100%')
      if (this.args.hint.length > 0) {
        Text(this.args.hint)
          .fontSize(Type.text3)
          .fontColor($r('app.color.fg3'))
          .textAlign(TextAlign.Center)
          .width('100%')
      }
      this.field(this.text, this.args.placeholder, 'prompt.field', (v: string) => {
        this.text = v;
      })
      if (this.args.secondPlaceholder.length > 0) {
        this.field(this.second, this.args.secondPlaceholder, 'prompt.second', (v: string) => {
          this.second = v;
        })
      }
      Row() {
        Text(t('mobile.cancel'))
          .fontSize(Type.text1)
          .fontColor($r('app.color.fg1'))
          .textAlign(TextAlign.Center)
          .layoutWeight(1)
          .height(Metrics.ctl)
          .onClick(() => this.args.close())
        Text(this.args.save.length > 0 ? this.args.save : t('mobile.save'))
          .id('prompt.save')
          .fontSize(Type.text1)
          .fontWeight(FontWeight.Medium)
          .fontColor(this.text.trim().length > 0 ? $r('app.color.fg_strong') : $r('app.color.fg3'))
          .textAlign(TextAlign.Center)
          .layoutWeight(1)
          .height(Metrics.ctl)
          .onClick(() => this.submit())
      }
      .width('100%')
    }
    .padding(Metrics.pad + Metrics.s1)
    .backgroundColor($r('app.color.menu_bg'))
    .borderRadius(FormMetrics.radius)
    .width('100%')
    .margin({ left: Metrics.page, right: Metrics.page })
    .constraintSize({ maxWidth: Metrics.contentW / 2 })
  }
}

@Builder
function promptBuilder(args: PromptArgs) {
  TextPrompt({ args: args })
}

/** Raises a text prompt over the page; its Save hands the text to `args.onSave` */
export function openTextPrompt(ui: UIContext, args: PromptArgs): void {
  const content = new ComponentContent<PromptArgs>(ui, wrapBuilder(promptBuilder), args);
  args.close = () => {
    ui.getPromptAction().closeCustomDialog(content);
  };
  // The promise rejects only without a window to raise it over
  ui.getPromptAction().openCustomDialog(content, { alignment: DialogAlignment.Center, autoCancel: true }).catch(() => {});
}

/** Asks before something with no way back (the chat's delete); the list's swipe deletes at once */
export function confirmDestructive(ui: UIContext, title: string, action: string, run: () => void): void {
  try {
    ui.showAlertDialog({
      title: title,
      message: '',
      autoCancel: true,
      alignment: DialogAlignment.Center,
      primaryButton: { value: t('mobile.cancel'), action: () => {} },
      secondaryButton: { value: action, fontColor: $r('app.color.danger'), action: run }
    });
  } catch (e) {
    // No dialog (a page without a window): nothing is deleted
  }
}
