import { LengthMetrics } from '@kit.ArkUI';
import { Metrics, WebMetrics, WebType } from '../design/Tokens';
import { Attachment } from '../wire/Blocks';
import { PromptRun, promptRuns } from '../logic/PromptMarks';
import { t } from '../store/L10n';
import { Lucide } from './Glyphs';

// The sent prompt (apps/ios Transcript/Cards.swift UserMessageCard + TurnAttachments, apps/android ui/Transcript.kt
// UserBubble, MediaViews.kt TurnAttachments): a right-aligned bubble on the bubble fill that always leaves `bubbleInset`
// free on its left, the advertised `/commands` and summoned `@personas` tinted as pills, the attachments packed to the
// right above it.

/** A quoted text's pill keeps this many characters */
const QUOTE_PILL: number = 24;

/** An attachment pill's label and glyph */
function pillLabel(a: Attachment): string {
  if (a.kind === 'selection') {
    const range = a.startLine === a.endLine ? `L${a.startLine ?? ''}` : `L${a.startLine ?? ''}-${a.endLine ?? ''}`;
    return `${a.name ?? ''} ${range}`;
  }
  if (a.kind === 'quote') {
    return (a.text ?? '').slice(0, QUOTE_PILL);
  }
  return a.name ?? '';
}

function pillIcon(a: Attachment): string {
  if (a.kind === 'selection') {
    return 'textSelect';
  }
  if (a.kind === 'quote') {
    return 'messageSquareText';
  }
  return /\.(mp4|mov|m4v|webm|mkv)$/i.test(a.name ?? '') ? 'fileVideo' : 'fileText';
}

/**
 * A prompt's attachments, wrapping and packed to the right: image tiles (the picture is the engine's blob, which the
 * phone fetches through the media layer; until then the tile shows its glyph), then file, text, selection and quote pills
 */
@ComponentV2
export struct TurnAttachments {
  @Param attachments: Attachment[] = [];

  build() {
    Flex({ wrap: FlexWrap.Wrap, justifyContent: FlexAlign.End, alignItems: ItemAlign.Center, space: { main: LengthMetrics.vp(Metrics.gap), cross: LengthMetrics.vp(Metrics.gap) } }) {
      ForEach(this.attachments, (a: Attachment) => {
        if (a.kind === 'image') {
          Stack() {
            Lucide({ name: 'image', tint: $r('app.color.fg3'), extent: Metrics.icon })
          }
          .width(WebMetrics.thumb)
          .height(WebMetrics.thumb)
          .backgroundColor($r('app.color.chip'))
          .borderRadius(Metrics.rSm)
          .accessibilityText(a.name ?? t('common.image'))
        } else {
          Row({ space: Metrics.s1 }) {
            Lucide({ name: pillIcon(a), tint: $r('app.color.fg3'), extent: WebMetrics.icon, weight: WebMetrics.strokeIcon })
            Text(pillLabel(a))
              .fontSize(WebType.text3)
              .lineHeight(WebType.text3Line)
              .fontWeight(FontWeight.Medium)
              .fontColor($r('app.color.fg2'))
              .maxLines(1)
              .textOverflow({ overflow: TextOverflow.Ellipsis })
            .ellipsisMode(EllipsisMode.CENTER)
              .flexShrink(1)
          }
          .height(WebMetrics.ctlSm)
          .padding({ left: Metrics.s2, right: Metrics.s2 })
          .backgroundColor($r('app.color.chip'))
          .borderRadius(Metrics.rSm)
          .constraintSize({ maxWidth: '100%' })
        }
      }, (a: Attachment, i: number) => `${i}:${a.kind}:${pillLabel(a)}`)
    }
    .width('100%')
  }
}

/**
 * The bubble itself: fg-strong text-1 on the bubble fill, `s3 + s0.5` × `s2.5` inside the bubble corners; an automatic
 * prompt (a /compact over the threshold, a continue) never draws
 */
@ComponentV2
export struct Bubble {
  @Param text: string = '';
  @Param attachments: Attachment[] = [];
  @Param auto: boolean = false;
  /** The names whose tokens get pills: the agent's advertised commands plus the one the turn recorded */
  @Param commands: string[] = [];
  /** The personas the agent can summon */
  @Param summons: string[] = [];

  build() {
    if (!this.auto) {
      Column({ space: Metrics.s2 }) {
        if (this.attachments.length > 0) {
          TurnAttachments({ attachments: this.attachments })
        }
        if (this.text.trim().length > 0) {
          Text() {
            ForEach(promptRuns(this.text.trimEnd(), this.commands, this.summons), (r: PromptRun) => {
              if (r.mark.length > 0) {
                Span(r.text)
                  .fontColor(r.mark === 'summon' ? $r('app.color.summon') : $r('app.color.command'))
                  .textBackgroundStyle({
                    color: r.mark === 'summon' ? $r('app.color.summon_pill') : $r('app.color.command_pill'),
                    radius: Metrics.rSm
                  })
              } else {
                Span(r.text)
              }
            }, (r: PromptRun, i: number) => `${i}:${r.mark}:${r.text}`)
          }
          .fontSize(WebType.text1)
          .lineHeight(WebType.text1Line)
          .fontColor($r('app.color.fg_strong'))
          .copyOption(CopyOptions.LocalDevice)
          .padding({
            left: Metrics.s3 + WebMetrics.s0_5, right: Metrics.s3 + WebMetrics.s0_5, top: WebMetrics.s2_5,
            bottom: WebMetrics.s2_5
          })
          .backgroundColor($r('app.color.bubble_bg'))
          .borderRadius(Metrics.rBubble)
        }
      }
      .alignItems(HorizontalAlign.End)
      .padding({ left: Metrics.bubbleInset })
      .width('100%')
    }
  }
}
