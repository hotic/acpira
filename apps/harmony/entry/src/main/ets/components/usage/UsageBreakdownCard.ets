import { FormMetrics, ListMetrics, Metrics, Type, UsageMetrics } from '../../design/Tokens';
import { AppModel } from '../../store/AppModel';
import { t } from '../../store/L10n';
import { folderLabel, projectName, remoteAlias } from '../../store/SessionLogic';
import {
  CHATS_KEY, figure, KEEP_ROWS, percent, UsageDim, UsageFigure, UsageModelLabels, UsagePart
} from '../../logic/UsageStats';
import { AgentMark, Lucide } from '../Glyphs';
import { makeSeg, SegChoice, UsageSectionHead, UsageSegmented } from './UsageCards';

// Settings → Usage's breakdown card (pages/Usage.ets): figures by model, agent or project

@ComponentV2
export struct UsageBreakdownCard {
  @Require @Param model: AppModel;
  @Param dim: UsageDim = 'model';
  @Param items: UsagePart[] = [];
  @Param labels: UsageModelLabels = new UsageModelLabels();
  @Param focusedKey: string = '';
  @Param wide: boolean = false;
  @Event onDim: (d: UsageDim) => void = () => {};
  @Event onPickFocus: (key: string) => void = () => {};
  @Local expanded: boolean = false;

  @Monitor('dim')
  onDimChange(): void {
    this.expanded = false;
  }

  private dimChoices(): SegChoice[] {
    return [
      makeSeg('model', t('usagePage.dim.model')),
      makeSeg('agent', t('usagePage.dim.agent')),
      makeSeg('project', t('usagePage.dim.project'))
    ];
  }

  private projectLabel(key: string): string {
    return key === CHATS_KEY ? t('session.group.chats') : folderLabel(key);
  }

  private rowName(key: string): string {
    if (this.dim === 'agent') {
      return this.model.agentName(key);
    }
    if (this.dim === 'project') {
      return this.projectLabel(key);
    }
    return this.labels.label.get(key)?.name ?? key;
  }

  private rowDetail(key: string): string {
    if (this.dim !== 'project' || key === CHATS_KEY) {
      return '';
    }
    const alias = remoteAlias(key);
    if (alias !== undefined) {
      return alias;
    }
    const label = this.projectLabel(key);
    const twin = this.items.some((p: UsagePart) => p.key !== key && p.key !== CHATS_KEY && this.projectLabel(p.key) === label);
    if (!twin) {
      return '';
    }
    const cut = Math.max(key.lastIndexOf('/'), key.lastIndexOf('\\'));
    return cut > 0 ? projectName(key.substring(0, cut)) : '';
  }

  private visibleItems(): UsagePart[] {
    const folds = this.items.length > KEEP_ROWS + 1;
    return !folds || this.expanded ? this.items : this.items.slice(0, KEEP_ROWS);
  }

  private foldedRest(): UsagePart[] {
    return this.items.length > KEEP_ROWS + 1 ? this.items.slice(KEEP_ROWS) : [];
  }

  private othersPart(): UsagePart {
    const rest = this.foldedRest();
    const out = new UsagePart();
    out.key = '*';
    for (let i = 0; i < rest.length; i++) {
      out.tokens += rest[i].tokens;
      out.share += rest[i].share;
    }
    return out;
  }

  @Builder
  dimControl() {
    UsageSegmented({
      choices: this.dimChoices(),
      selected: this.dim,
      onPick: (v: string) => this.onDim(v as UsageDim)
    })
  }

  @Builder
  markSlot(key: string, isMore: boolean) {
    Row() {
      if (isMore) {
        Lucide({ name: 'ellipsis', tint: $r('app.color.fg3'), extent: Metrics.icon })
      } else if (this.dim === 'model') {
        AgentMark({ agent: this.labels.label.get(key)?.brand ?? '', name: this.rowName(key), tint: $r('app.color.fg2'), extent: Metrics.icon })
      } else if (this.dim === 'agent') {
        AgentMark({ agent: key, name: this.rowName(key), tint: $r('app.color.fg2'), extent: Metrics.icon })
      } else if (key === CHATS_KEY) {
        Lucide({ name: 'messageSquare', tint: $r('app.color.fg2'), extent: Metrics.icon })
      } else if (remoteAlias(key) !== undefined) {
        Stack({ alignContent: Alignment.BottomEnd }) {
          Lucide({ name: 'folder', tint: $r('app.color.fg2'), extent: Metrics.icon })
          Lucide({ name: 'globe', tint: $r('app.color.fg2'), extent: UsageMetrics.crown })
        }
        .width(Metrics.icon)
        .height(Metrics.icon)
      } else {
        Lucide({ name: 'folder', tint: $r('app.color.fg2'), extent: Metrics.icon })
      }
    }
    .width(Metrics.lead)
    .height(Metrics.lead)
    .justifyContent(FlexAlign.Center)
  }

  @Builder
  shareTrack(share: number, muted: boolean) {
    Stack({ alignContent: Alignment.Start }) {
      Row()
        .width('100%')
        .height(UsageMetrics.trackH)
        .borderRadius(UsageMetrics.trackH / 2)
        .backgroundColor($r('app.color.usage_track'))
      Row()
        .width(`${Math.max(0, Math.min(100, share * 100))}%`)
        .height(UsageMetrics.trackH)
        .borderRadius(UsageMetrics.trackH / 2)
        .backgroundColor(muted ? $r('app.color.usage_dim') : $r('app.color.usage_ink'))
    }
    .height(UsageMetrics.trackH)
    .width('100%')
  }

  @Builder
  tokenReadout(fig: UsageFigure) {
    Row({ space: ListMetrics.s0_5 }) {
      Text(fig.value)
        .fontSize(Type.text3)
        .fontColor($r('app.color.fg1'))
        .maxLines(1)
      if (fig.unit !== undefined) {
        Text(fig.unit)
          .fontSize(Type.text4)
          .fontColor($r('app.color.fg3'))
          .maxLines(1)
      }
    }
    .justifyContent(FlexAlign.End)
    .alignItems(VerticalAlign.Bottom)
  }

  @Builder
  breakdownRow(part: UsagePart, isMore: boolean, moreCount: number) {
    Row({ space: Metrics.gap }) {
      this.markSlot(part.key, isMore)
      if (this.wide) {
        Row({ space: ListMetrics.s1_5 }) {
          Text(isMore ? t(`usagePage.more.${this.dim}`, new Map<string, string>([['n', `${moreCount}`]])) : this.rowName(part.key))
            .fontSize(Type.text3)
            .fontColor(isMore ? $r('app.color.fg2') : $r('app.color.fg1'))
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
            .flexShrink(1)
          if (!isMore && this.rowDetail(part.key).length > 0) {
            Text(this.rowDetail(part.key))
              .fontSize(Type.text4)
              .fontColor($r('app.color.fg3'))
              .maxLines(1)
          }
        }
        .width(UsageMetrics.nameWide)
        Row() {
          this.shareTrack(part.share, isMore)
        }
        .layoutWeight(1)
        Row() {
          this.tokenReadout(figure(part.tokens))
        }
        .width(UsageMetrics.numW)
        .justifyContent(FlexAlign.End)
        Text(percent(part.share))
          .fontSize(Type.text4)
          .fontColor($r('app.color.fg3'))
          .textAlign(TextAlign.End)
          .width(UsageMetrics.pctW)
          .maxLines(1)
      } else {
        Column({ space: ListMetrics.s1_5 }) {
          Row({ space: Metrics.s2 }) {
            Row({ space: ListMetrics.s1_5 }) {
              Text(isMore ? t(`usagePage.more.${this.dim}`, new Map<string, string>([['n', `${moreCount}`]])) : this.rowName(part.key))
                .fontSize(Type.text3)
                .fontColor(isMore ? $r('app.color.fg2') : $r('app.color.fg1'))
                .maxLines(1)
                .textOverflow({ overflow: TextOverflow.Ellipsis })
                .flexShrink(1)
              if (!isMore && this.rowDetail(part.key).length > 0) {
                Text(this.rowDetail(part.key))
                  .fontSize(Type.text4)
                  .fontColor($r('app.color.fg3'))
                  .maxLines(1)
              }
            }
            .layoutWeight(1)
            this.tokenReadout(figure(part.tokens))
            Text(percent(part.share))
              .fontSize(Type.text4)
              .fontColor($r('app.color.fg3'))
              .textAlign(TextAlign.End)
              .width(UsageMetrics.pctW)
              .maxLines(1)
          }
          .width('100%')
          this.shareTrack(part.share, isMore)
        }
        .layoutWeight(1)
      }
    }
    .width('100%')
    .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s3, bottom: Metrics.s3 })
    .opacity(this.focusedKey.length > 0 && this.focusedKey !== part.key ? UsageMetrics.dimRow : 1)
    .onClick(() => {
      if (isMore) {
        this.onPickFocus('');
        this.expanded = true;
      } else {
        this.onPickFocus(this.focusedKey === part.key ? '' : part.key);
      }
    })
  }

  build() {
    Column({ space: UsageMetrics.groupHeadGap }) {
      UsageSectionHead({
        title: t('usagePage.breakdown'),
        count: this.items.length,
        trailing: () => { this.dimControl(); }
      })
      if (this.items.length > 0) {
        Column() {
          ForEach(this.visibleItems(), (part: UsagePart, idx: number) => {
            Column() {
              if (idx > 0) {
                Divider().color($r('app.color.line')).strokeWidth(Metrics.hairline).margin({ left: Metrics.pad, right: Metrics.pad })
              }
              this.breakdownRow(part, false, 0)
            }
            .width('100%')
          }, (part: UsagePart) => `${part.key}:${part.tokens}`)
          if (this.foldedRest().length > 0 && !this.expanded) {
            Divider().color($r('app.color.line')).strokeWidth(Metrics.hairline).margin({ left: Metrics.pad, right: Metrics.pad })
            this.breakdownRow(this.othersPart(), true, this.foldedRest().length)
          }
        }
        .width('100%')
        .borderRadius(FormMetrics.radius)
        .backgroundColor($r('app.color.form_row'))
        .clip(true)
      }
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
  }
}
