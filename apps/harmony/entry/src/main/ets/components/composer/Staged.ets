import { pasteboard } from '@kit.BasicServicesKit';
import { Attachment } from '../../wire/Wire';
import { Cmp, Metrics, Type } from '../../design/Tokens';
import { PastedText } from '../../logic/Composing';
import { ComposerDraft } from '../../store/Composing';
import { t } from '../../store/L10n';
import { Lucide } from '../Glyphs';

// The attachments waiting in the composer (apps/ios Chat/Composer.swift StagedAttachments, Chat/Quotes.swift QuoteChip /
// QuoteCard, Chat/TextAttachment.swift): photo tiles and file pills over the text, the quotes as one annotations chip
// with their card, and a pasted text's pill that opens its contents.

/** A staged chip's ✕, riding its top-right corner (RemoveBadge) */
@ComponentV2
export struct RemoveBadge {
  @Param label: string = '';
  @Event onTap: () => void = () => {};

  build() {
    Stack() {
      Lucide({ name: 'x', tint: $r('app.color.btn1_fg'), extent: Cmp.chevron })
    }
    .width(Metrics.lead)
    .height(Metrics.lead)
    .borderRadius(Metrics.lead / 2)
    .backgroundColor($r('app.color.btn1_bg'))
    .accessibilityText(this.label)
    // Riding the corner of the chip it sits on (a Stack aligned TopEnd)
    .translate({ x: Cmp.s1_5, y: -Cmp.s1_5 })
    .onClick(() => this.onTap())
  }
}

/** An attachment's pill (AttachmentPill): its glyph and name, a tap opens it, and a ✕ after the name while staged */
@ComponentV2
export struct AttachmentPill {
  @Param name: string = '';
  @Param icon: string = 'fileText';
  @Param removable: boolean = false;
  @Event onOpen: () => void = () => {};
  @Event onRemove: () => void = () => {};

  build() {
    Row({ space: Metrics.s1 }) {
      Row({ space: Metrics.s1 }) {
        Lucide({ name: this.icon, tint: $r('app.color.fg3'), extent: Cmp.webIcon })
        Text(this.name)
          .fontSize(Type.text3)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg2'))
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
          .constraintSize({ maxWidth: Cmp.stagedFileW })
      }
      .onClick(() => this.onOpen())
      if (this.removable) {
        Stack() {
          Lucide({ name: 'x', tint: $r('app.color.fg3'), extent: Cmp.chevron })
        }
        .width(Cmp.webCtlSm)
        .height(Cmp.webCtlSm)
        .margin({ right: (Cmp.chevron - Cmp.webCtlSm) / 2 })
        .accessibilityText(t('common.remove'))
        .onClick(() => this.onRemove())
      }
    }
    .height(Cmp.webCtlSm)
    .padding({ left: Metrics.s2, right: Metrics.s2 })
    .borderRadius(Metrics.rSm)
    .backgroundColor($r('app.color.chip'))
  }
}

/** The strip inside the card over the text: quotes first as one chip, then photo tiles and file / text pills */
@ComponentV2
export struct StagedStrip {
  @Require @Param draft: ComposerDraft;
  /** `quotes`: the quote card open on appear (screenshots) */
  @Param popup: string = '';

  private hasQuotes(): boolean {
    return this.draft.staged.some((a: Attachment) => a.kind === 'quote');
  }

  private remove(i: number): void {
    this.draft.staged = this.draft.staged.filter((_: Attachment, j: number) => j !== i);
  }

  build() {
    Scroll() {
      Row({ space: Metrics.s2 }) {
        if (this.hasQuotes()) {
          Stack({ alignContent: Alignment.TopEnd }) {
            QuoteChip({ draft: this.draft, startOpen: this.popup === 'quotes' })
            RemoveBadge({ label: t('quote.removeAll'), onTap: () => {
              this.draft.staged = this.draft.staged.filter((a: Attachment) => a.kind !== 'quote');
            } })
          }
        }
        ForEach(this.draft.staged, (a: Attachment, i: number) => {
          if (a.kind === 'image') {
            Stack({ alignContent: Alignment.TopEnd }) {
              Stack() {
                if (a.thumb !== undefined) {
                  Image(a.thumb)
                    .objectFit(ImageFit.Cover)
                    .width(Cmp.stagedTile)
                    .height(Cmp.stagedTile)
                } else {
                  Lucide({ name: 'image', tint: $r('app.color.fg3'), extent: Metrics.icon })
                }
              }
              .width(Cmp.stagedTile)
              .height(Cmp.stagedTile)
              .borderRadius(Metrics.rSm)
              .clip(true)
              .backgroundColor($r('app.color.chip'))
              .border({ width: Metrics.hairline, color: $r('app.color.line') })
              RemoveBadge({ label: t('common.remove'), onTap: () => this.remove(i) })
            }
          } else if (a.kind === 'text') {
            TextAttachmentChip({ blob: a.blob, name: a.name ?? t('attach.pastedText'), removable: true,
              onRemove: () => this.remove(i) })
          } else if (a.kind === 'file') {
            AttachmentPill({ name: a.name ?? '', icon: 'fileText', removable: true, onRemove: () => this.remove(i) })
          } else if (a.kind === 'selection') {
            AttachmentPill({
              name: `${a.name ?? ''} ${a.startLine === a.endLine ? `L${a.startLine ?? 0}` : `L${a.startLine ?? 0}-${a.endLine ?? 0}`}`,
              icon: 'textSelect', removable: true, onRemove: () => this.remove(i)
            })
          }
        }, (a: Attachment, i: number) => `${i}:${a.kind}:${a.name ?? a.uri ?? ''}`)
      }
      .alignItems(VerticalAlign.Bottom)
      .padding({ top: Cmp.s1_5, right: Cmp.s1_5, left: Metrics.s1 })
    }
    .scrollable(ScrollDirection.Horizontal)
    .scrollBar(BarState.Off)
    .align(Alignment.Start)
    .width('100%')
  }
}

/**
 * The staged quotes as one chip (Quotes.swift QuoteChip): a tap opens their card, where each shows its text and takes a
 * comment or leaves
 */
@ComponentV2
export struct QuoteChip {
  @Require @Param draft: ComposerDraft;
  @Param startOpen: boolean = false;
  @Local open: boolean = false;

  aboutToAppear(): void {
    this.open = this.startOpen;
  }

  private places(): number[] {
    const out: number[] = [];
    this.draft.staged.forEach((a: Attachment, i: number) => {
      if (a.kind === 'quote') {
        out.push(i);
      }
    });
    return out;
  }

  build() {
    Row({ space: Metrics.s1 }) {
      Lucide({ name: 'messageSquareText', tint: $r('app.color.fg3'), extent: Cmp.webIcon })
      Text(this.places().length === 1 ? t('quote.chipOne') :
        t('quote.chip', new Map<string, string>([['n', `${this.places().length}`]])))
        .fontSize(Type.text3)
        .fontWeight(FontWeight.Medium)
        .fontColor($r('app.color.fg2'))
        .maxLines(1)
    }
    .height(Cmp.webCtlSm)
    .padding({ left: Metrics.s2, right: Metrics.s2 })
    .borderRadius(Metrics.rSm)
    .backgroundColor($r('app.color.chip'))
    .onClick(() => {
      this.open = true;
    })
    .bindPopup(this.open, {
      builder: () => {
        this.card();
      },
      placement: Placement.Top,
      popupColor: $r('app.color.menu_bg'),
      backgroundBlurStyle: BlurStyle.NONE,
      radius: Cmp.rGooPanel,
      enableArrow: false,
      onStateChange: (e) => {
        if (!e.isVisible) {
          this.open = false;
        }
      }
    })
  }

  /** The quotes' card: numbered entries, each its text (five lines at most), a comment field and its removal */
  @Builder
  card() {
    Scroll() {
      Column({ space: Cmp.s3 }) {
        ForEach(this.places(), (at: number, n: number) => {
          Row({ space: Metrics.gapHalf }) {
            Text(`${n + 1}.`)
              .fontSize(Type.text3)
              .fontColor($r('app.color.fg3'))
              .fontFeature('"tnum" on')
            Column({ space: Cmp.s0_5 }) {
              Text(t('quote.selected'))
                .fontSize(Type.text3)
                .fontColor($r('app.color.fg3'))
              Text(this.draft.staged[at]?.text ?? '')
                .fontSize(Type.text2)
                .fontColor($r('app.color.fg1'))
                .maxLines(Cmp.quoteLines)
                .textOverflow({ overflow: TextOverflow.Ellipsis })
              TextArea({ text: this.draft.staged[at]?.comment ?? '', placeholder: t('quote.commentPlaceholder') })
                .fontSize(Type.text2)
                .fontColor($r('app.color.fg_strong'))
                .placeholderColor($r('app.color.fg3'))
                .placeholderFont({ size: Type.text2 })
                .backgroundColor(Color.Transparent)
                .padding(0)
                .borderRadius(0)
                .maxLines(Cmp.quoteCommentLines)
                .margin({ top: Metrics.s1 })
                .accessibilityText(t('quote.comment'))
                .onChange((value: string) => this.comment(at, value))
            }
            .alignItems(HorizontalAlign.Start)
            .layoutWeight(1)
            Stack() {
              Lucide({ name: 'trash2', tint: $r('app.color.fg2'), extent: Metrics.icon })
            }
            .width(Cmp.ctlSm)
            .height(Cmp.ctlSm)
            .accessibilityText(t('quote.remove'))
            .onClick(() => this.remove(at))
          }
          .alignItems(VerticalAlign.Top)
          .width('100%')
        }, (at: number, n: number) => `${n}:${at}`)
      }
      .padding(Cmp.s3)
    }
    .scrollBar(BarState.Off)
    .width(Cmp.popLg)
    .constraintSize({ maxHeight: Cmp.popLg })
  }

  /** A quote's comment as the field edits it; cleared, the quote goes out without one */
  private comment(at: number, value: string): void {
    const a = this.draft.staged[at];
    if (a === undefined || a.kind !== 'quote') {
      return;
    }
    const next = this.draft.staged.slice();
    const quote: Attachment = { kind: 'quote', text: a.text, comment: value.length > 0 ? value : undefined };
    next[at] = quote;
    this.draft.staged = next;
  }

  private remove(at: number): void {
    const last = this.places().length === 1;
    this.draft.staged = this.draft.staged.filter((_: Attachment, j: number) => j !== at);
    if (last) {
      this.open = false;
    }
  }
}

/** A pasted text's pill (TextAttachmentChip): a tap opens its contents in a sheet with a copy */
@ComponentV2
export struct TextAttachmentChip {
  @Param blob: string | undefined = undefined;
  @Param name: string = '';
  @Param removable: boolean = false;
  @Event onRemove: () => void = () => {};
  @Local open: boolean = false;

  build() {
    AttachmentPill({ name: this.name, icon: 'fileText', removable: this.removable, onOpen: () => {
      this.open = true;
    }, onRemove: () => this.onRemove() })
      // A wide window hangs a sheet off its host as a popup by default: centred there, the bottom sheet on a phone as before
      .bindSheet(this.open, this.sheet(), {
        preferType: SheetType.CENTER,
        height: SheetSize.LARGE,
        backgroundColor: $r('app.color.bg0'),
        title: { title: this.name },
        onDisappear: () => {
          this.open = false;
        }
      })
  }

  /** The original contents (a text still held as base64; one the engine stored is not loaded here) */
  @Builder
  sheet() {
    Column({ space: Metrics.s2 }) {
      if (PastedText.inline(this.blob) !== undefined) {
        Row() {
          Blank()
          Row({ space: Metrics.s1 }) {
            Lucide({ name: 'copy', tint: $r('app.color.fg2'), extent: Metrics.icon })
            Text(t('common.copy'))
              .fontSize(Type.text3)
              .fontColor($r('app.color.fg2'))
          }
          .onClick(() => {
            const data = pasteboard.createData(pasteboard.MIMETYPE_TEXT_PLAIN, PastedText.inline(this.blob) ?? '');
            pasteboard.getSystemPasteboard().setDataSync(data);
          })
        }
        .width('100%')
        Scroll() {
          Text(PastedText.inline(this.blob) ?? '')
            .fontSize(Type.body)
            .lineHeight(Type.bodyLine)
            .fontColor($r('app.color.fg_strong'))
            .copyOption(CopyOptions.LocalDevice)
            .width('100%')
        }
        .scrollBar(BarState.Off)
        .layoutWeight(1)
        .align(Alignment.Top)
      } else {
        Text(t('attach.loadFailed', new Map<string, string>([['name', this.name]])))
          .fontSize(Type.text2)
          .fontColor($r('app.color.fg3'))
      }
    }
    .padding({ left: Metrics.pad, right: Metrics.pad, bottom: Metrics.pad })
    .height('100%')
    .width('100%')
  }
}
