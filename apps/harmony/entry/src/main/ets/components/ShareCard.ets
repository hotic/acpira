import { image } from '@kit.ImageKit';
import { photoAccessHelper } from '@kit.MediaLibraryKit';
import { fileIo } from '@kit.CoreFileKit';
import { Metrics, WebMetrics, WebType } from '../design/Tokens';
import { Block, TurnX } from '../wire/Blocks';
import { splitCodexBlocks } from '../logic/Folding';
import { ModelCatalog, vendorBrand } from '../logic/ModelCatalog';
import { t } from '../store/L10n';
import { AgentMark, Lucide } from './Glyphs';
import { Prose } from './Prose';

// apps/ios Transcript/ShareCard.swift (SharePreview.tsx ShareCard and shareCapture.ts): a reply as a watermarked
// picture. The card is drawn off screen by the component snapshot and shown whole over the chat, where the system's
// save button puts it in the gallery. Vendor colours are not ported, so the mark keeps the neutral ink and the frame
// has no corner glow.

/** The watermark line's facts (turnActionHelpers.ts ShareMeta) */
export class ShareMeta {
  title: string = '';
  agent: string = '';
  /** The model that answered, empty when nothing names it: the agent stands in */
  model: string = '';
  brand: string = '';
}

interface ShareOption {
  id: string;
  name: string;
}

interface ShareConfig {
  category?: string;
  value?: string;
  options?: ShareOption[];
}

interface ShareControls {
  options?: ShareConfig[];
}

/** The option a reported id stands for: an exact match, else the only one differing by its `[…]` hint (Claude reports
 * `fable[1m]` for an offered `fable`), as the webview's offeredModel matches it */
function offeredOption(options: ShareOption[], id: string): ShareOption | undefined {
  const exact = options.find((o: ShareOption) => o.id === id);
  if (exact !== undefined) return exact;
  const bare = (s: string): string => s.endsWith(']') && s.lastIndexOf('[') > 0 ? s.substring(0, s.lastIndexOf('[')) : s;
  const same = options.filter((o: ShareOption) => bare(o.id) === bare(id));
  return same.length === 1 ? same[0] : undefined;
}

/** turnShareMeta: the peer's own model id when it reported one, else the session's current model, named by its family */
export function shareMeta(title: string, agent: string, controls: Object | undefined, turn: TurnX): ShareMeta {
  const meta = new ShareMeta();
  meta.title = title;
  meta.agent = agent;
  const config = ((controls as ShareControls | undefined)?.options ?? []).find((c: ShareConfig) => c.category === 'model');
  const id = turn.usage?.model ?? config?.value ?? '';
  const option = offeredOption(config?.options ?? [], id);
  if (option !== undefined) {
    // The family alone, as the statistics name it; a Fusion pair keeps its whole name
    const parsed = ModelCatalog.parse(option.name);
    meta.model = parsed.lead === undefined ? parsed.family : option.name;
    meta.brand = vendorBrand(option.id) ?? vendorBrand(option.name) ?? '';
  } else if (id.length > 0) {
    meta.model = id;
    meta.brand = vendorBrand(id) ?? '';
  }
  return meta;
}

/** What a reply's picture holds: its prose, or (a tail the split finds nothing in) the turn's plain text */
export function shareBlocks(turn: TurnX): Block[] {
  const blocks = (turn.blocks ?? []) as Block[];
  const filled = (b: Block): boolean => (b.markdown ?? '').trim().length > 0;
  const reply = splitCodexBlocks(blocks).reply.filter(filled);
  return reply.length > 0 ? reply : blocks.filter((b: Block) => b.type === 'text' && filled(b));
}

class ShareTokens {
  static readonly watermark: number = 0.85;
  /** The light sheet's lift off the frame; `share_lift` is clear in the dark table */
  static readonly liftRadius: number = 20;
  static readonly liftY: number = 8;
  /** Before the snapshot is taken: the card's first layout */
  static readonly settleMs: number = 300;
  static readonly pngQuality: number = 100;
}

/** What the off-screen builder draws */
export class SharePayload {
  meta: ShareMeta = new ShareMeta();
  blocks: Block[] = [];
  /** The thread's width, vp */
  cardWidth: number = 0;
}

/**
 * The card in two layers: the reply on a raised sheet, the frame under it carrying one line that names the session on
 * the left and the model on the right
 */
@ComponentV2
export struct ShareCard {
  @Param payload: SharePayload = new SharePayload();

  build() {
    Column() {
      Column({ space: Metrics.gap }) {
        ForEach(this.payload.blocks, (b: Block) => {
          Prose({ markdown: b.markdown ?? '' })
        }, (b: Block, i: number) => `${i}`)
      }
      .alignItems(HorizontalAlign.Start)
      .padding(Metrics.pad)
      .width('100%')
      .backgroundColor($r('app.color.share_sheet'))
      .borderRadius(Metrics.rMd)
      .border({ width: Metrics.hairline, color: $r('app.color.share_sheet_line') })
      .shadow({ radius: ShareTokens.liftRadius, color: $r('app.color.share_lift'), offsetY: ShareTokens.liftY })

      Row({ space: Metrics.gap }) {
        Row({ space: Metrics.s1 }) {
          AgentMark({ agent: 'acpira', name: 'Acpira', tint: $r('app.color.fg3'), extent: WebMetrics.icon })
          Text('Acpira')
            .fontSize(WebType.text4)
            .lineHeight(WebType.text4Line)
            .fontWeight(FontWeight.Medium)
            .fontColor($r('app.color.fg2'))
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
            .layoutWeight(1)
        }
        .layoutWeight(1)

        Row({ space: Metrics.s1 }) {
          AgentMark({
            agent: this.payload.meta.model.length > 0 ? this.payload.meta.brand : this.payload.meta.agent,
            name: this.payload.meta.model.length > 0 ? this.payload.meta.model : this.payload.meta.agent,
            tint: this.payload.meta.model.length > 0 ? $r('app.color.fg2') : $r('app.color.fg3'), extent: WebMetrics.icon
          })
          Text(this.payload.meta.model.length > 0 ? this.payload.meta.model : this.payload.meta.agent)
            .fontSize(WebType.text4)
            .lineHeight(WebType.text4Line)
            .fontWeight(FontWeight.Medium)
            .fontColor($r('app.color.fg2'))
            .maxLines(1)
        }
      }
      .opacity(ShareTokens.watermark)
      .padding({ top: Metrics.pad, bottom: Metrics.gap })
      .width('100%')
    }
    // The frame (.acp-share-card px/pt-share-frame, pb-gap-half): gap × 5/4 on the sides and top
    .padding({ left: Metrics.gap * 5 / 4, right: Metrics.gap * 5 / 4, top: Metrics.gap * 5 / 4, bottom: WebMetrics.s1_5 })
    .width(this.payload.cardWidth)
    .backgroundColor($r('app.color.share_frame'))
    .borderRadius(Metrics.rLg)
    .border({ width: Metrics.hairline, color: $r('app.color.conversation_line') })
    .clip(true)
  }
}

@Builder
function shareCardBuilder(payload: SharePayload) {
  ShareCard({ payload: payload })
}

/** The card as a picture; undefined where nothing can draw it (the Previewer) */
export async function captureShare(ui: UIContext, payload: SharePayload): Promise<image.PixelMap | undefined> {
  try {
    return await ui.getComponentSnapshot().createFromBuilder(() => {
      shareCardBuilder(payload)
    }, ShareTokens.settleMs, true);
  } catch (e) {
    return undefined;
  }
}

/** Writes the picture into the gallery as a PNG; called from the save button's granted tap */
async function savePicture(ui: UIContext, picture: image.PixelMap): Promise<boolean> {
  if (!canIUse('SystemCapability.FileManagement.PhotoAccessHelper.Core')) {
    return false;
  }
  try {
    const helper = photoAccessHelper.getPhotoAccessHelper(ui.getHostContext());
    const uri = await helper.createAsset(photoAccessHelper.PhotoType.IMAGE, 'png');
    const file = await fileIo.open(uri, fileIo.OpenMode.READ_WRITE);
    const packer = image.createImagePacker();
    try {
      await packer.packToFile(picture, file.fd, { format: 'image/png', quality: ShareTokens.pngQuality });
    } finally {
      await packer.release();
      await fileIo.close(file);
    }
    return true;
  } catch (e) {
    return false;
  }
}

/**
 * The picture whole over the chat, on black as the other apps' Lightbox shows it: close on the left, the system's save
 * button on the right (a security component: its tap grants the one write, no permission asked)
 */
@ComponentV2
export struct SharePreview {
  @Param picture: image.PixelMap | undefined = undefined;
  @Event onClose: () => void = () => {};
  @Local saved: boolean = false;

  build() {
    Stack({ alignContent: Alignment.Top }) {
      if (this.picture !== undefined) {
        Image(this.picture)
          .objectFit(ImageFit.Contain)
          .width('100%')
          .height('100%')
          .onClick(() => this.onClose())
      }
      Row() {
        Stack() {
          Lucide({ name: 'x', tint: Color.White, extent: Metrics.icon })
        }
        .width(WebMetrics.ctl)
        .height(WebMetrics.ctl)
        .accessibilityText(t('common.close'))
        .onClick(() => this.onClose())

        Blank()
        if (this.saved) {
          Stack() {
            Lucide({ name: 'check', tint: Color.White, extent: Metrics.icon })
          }
          .width(WebMetrics.ctl)
          .height(WebMetrics.ctl)
        } else {
          SaveButton({ icon: SaveIconStyle.LINES, buttonType: ButtonType.Circle })
            .iconColor(Color.White)
            .backgroundColor(Color.Transparent)
            .width(WebMetrics.ctl)
            .height(WebMetrics.ctl)
            .onClick((_: ClickEvent, result: SaveButtonOnClickResult) => {
              const picture = this.picture;
              if (result !== SaveButtonOnClickResult.SUCCESS || picture === undefined) {
                return;
              }
              savePicture(this.getUIContext(), picture).then((ok: boolean) => {
                this.saved = ok;
              });
            })
        }
      }
      .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2 })
      .width('100%')
    }
    .width('100%')
    .height('100%')
    .backgroundColor(Color.Black)
  }
}
