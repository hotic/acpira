import { i18n } from '@kit.LocalizationKit';
import { Metrics } from '../design/Tokens';
import { fixtureView } from '../store/Demo';
import { initL10n } from '../store/L10n';
import { TurnX } from '../wire/Blocks';
import { SessionView } from '../wire/Wire';
import { shareBlocks, ShareCard, shareMeta, SharePayload } from '../components/ShareCard';

// The Previewer takes no taps and draws no component snapshot, so a reply's screenshot card stands here as the preview
// shows it: the showcase's last reply on black (scripts/preview.sh … pages/PreviewShare). Not reached from the app.

interface ViewControls {
  controls?: Object;
}

function payloadOf(fixture: string, cardWidth: number): SharePayload {
  const payload = new SharePayload();
  const view: SessionView | undefined = fixtureView(fixture);
  if (view === undefined) {
    return payload;
  }
  const turns = view.turns as TurnX[];
  const turn = turns.slice().reverse().find((x: TurnX) => x.role === 'agent');
  if (turn === undefined) {
    return payload;
  }
  payload.meta = shareMeta(view.title, view.agent, (view as Object as ViewControls).controls, turn);
  payload.blocks = shareBlocks(turn);
  payload.cardWidth = cardWidth;
  return payload;
}

@Entry
@ComponentV2
struct PreviewShare {
  @Local payload: SharePayload = new SharePayload();
  private room: number = 0;

  aboutToAppear(): void {
    initL10n(i18n.System.getSystemLanguage());
  }

  build() {
    Column() {
      if (this.payload.cardWidth > 0) {
        ShareCard({ payload: this.payload })
      }
    }
    .justifyContent(FlexAlign.Center)
    .height('100%')
    .width('100%')
    .backgroundColor(Color.Black)
    .onAreaChange((_: Area, area: Area) => {
      if (this.room === 0) {
        this.room = Number(area.width);
        this.payload = payloadOf('showcase', Math.min(this.room, Metrics.contentW));
      }
    })
  }
}
