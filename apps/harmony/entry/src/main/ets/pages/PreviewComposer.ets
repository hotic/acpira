import { i18n } from '@kit.LocalizationKit';
import { Attachment, Turn } from '../wire/Wire';
import { AppModel } from '../store/AppModel';
import { initL10n, t } from '../store/L10n';
import { fixtureView } from '../store/Demo';
import { ChatPage } from './ChatPage';
import { NewSession } from './NewSession';
import { ChatBarActions } from '../components/ChatBar';

// The Previewer cannot tap, so the composer's and the dock's states show here as phone-wide frames that wrap into rows
// (scripts/preview.sh <out> 3780 3510 dark pages/PreviewComposer 240): each frame a demo model on a fixture, in the state
// the iOS screenshots name with `-popup` (apps/ios/scripts/shot.sh). Not reached from the app's own pages.

/** A phone's width and height in vp (1080 × 2340 px at 480 dpi) */
const PHONE_W: number = 360;
const PHONE_H: number = 780;

/** Each frame: its fixture and its popup */
const SCENES: string[][] = [
  ['running', ''], ['queued', ''], ['question', ''], ['failure', ''], ['context', 'context'], ['goal', ''], ['plan', ''],
  ['showcase', 'model'], ['showcase', 'model.2'], ['showcase', 'attach'], ['showcase', 'focus'], ['builtin', 'model'],
  ['builtin', 'model.0.sidekick'], ['fusion', 'model'],
  ['media', 'staged'], ['showcase', 'quotes'], ['showcase', 'edit'], ['showcase', 'slash'], ['showcase', 'mention'],
  ['showcase', ''], ['new', 'model']
];

/** A frame: a chat on a fixture with the composer in one state */
@ComponentV2
struct Scene {
  @Param fixture: string = 'showcase';
  @Param popup: string = '';
  @Local model: AppModel = new AppModel();

  aboutToAppear(): void {
    const model = new AppModel();
    model.attach(undefined, i18n.System.getSystemLanguage());
    model.demo();
    if (this.fixture !== 'new') {
      model.open(model.summaries[1]?.id ?? '');
      model.view = fixtureView(this.fixture);
    }
    const id = model.view?.id ?? '';
    const draft = model.draftFor(id);
    // `model`, `model.2`, `attach`, `model.0.sidekick`: a chip's beans out, a panel open, the sidekick's level
    const parts = this.popup.split('.');
    if (parts[0] === 'model' || parts[0] === 'attach') {
      const goo = this.fixture === 'new' ? model.newGoo : model.goo;
      goo.open = parts[0];
      goo.last = parts[0];
      goo.stage = parts.length > 1 ? Number(parts[1]) : 0;
      goo.deeper = parts.length > 2 ? parts[2] : undefined;
    }
    if (this.popup === 'staged') {
      const file: Attachment = { kind: 'file', uri: 'file:///Users/dev/acpira/Composer.swift', name: 'Composer.swift' };
      const other: Attachment = { kind: 'file', uri: 'file:///Users/dev/acpira/ios.md', name: 'ios.md' };
      const pasted: Attachment = { kind: 'text', blob: 'cGFzdGVk', name: t('attach.pastedText') };
      draft.staged = [file, pasted, other];
    }
    if (this.popup === 'quotes') {
      const a: Attachment = { kind: 'quote', text: 'src/foo.ts 的旧解析已经没有调用方了，a.ts 和 b.ts 都改成调用 parseArgs。' };
      const b: Attachment = { kind: 'quote', text: 'README 补上了 --dry-run 的说明', comment: '示例也要更新' };
      draft.staged = [a, b];
    }
    if (this.popup === 'edit') {
      const turns = model.view?.turns ?? [];
      let last = -1;
      turns.forEach((turn: Turn, i: number) => {
        if (turn.role === 'user') {
          last = i;
        }
      });
      model.beginEdit(last);
    }
    if (this.popup === 'slash') {
      draft.text = '/';
    }
    if (this.popup === 'mention') {
      draft.text = '@src';
    }
    this.model = model;
  }

  build() {
    Stack() {
      if (this.fixture === 'new') {
        NewSession({ model: this.model })
      } else {
        ChatPage({
          model: this.model, showBack: true, actions: new ChatBarActions(),
          popup: this.popup === 'slash' || this.popup === 'mention' ? 'focus' : this.popup
        })
      }
    }
    .width(PHONE_W)
    .height(PHONE_H)
    .clip(true)
    .backgroundColor($r('app.color.bg0'))
  }
}

@Entry
@ComponentV2
struct PreviewComposer {
  aboutToAppear(): void {
    initL10n(i18n.System.getSystemLanguage());
  }

  build() {
    Flex({ wrap: FlexWrap.Wrap }) {
      ForEach(SCENES, (scene: string[]) => {
        Scene({ fixture: scene[0], popup: scene[1] })
      }, (scene: string[]) => scene.join(':'))
    }
    .width('100%')
    .height('100%')
  }
}
