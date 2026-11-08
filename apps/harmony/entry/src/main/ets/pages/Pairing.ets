import { promptAction } from '@kit.ArkUI';
import { Metrics, PairMetrics, Type } from '../design/Tokens';
import { Lucide } from '../components/Glyphs';
import { PairHero, PairHeroPlacement } from '../components/PairHero';
import { AppModel } from '../store/AppModel';
import { t } from '../store/L10n';
import { PairingModel } from '../store/PairingModel';
import { PairedComputer } from '../remote/Stores';

// The phone's side of paired remote access on screen (apps/ios Pairing/PairListViews.swift + PairSettings.swift,
// apps/android ui/Pairing.kt): the connect frame in the list's place while there is nothing to list, and the computers
// page (every paired computer with how it is reached, the followed one ticked; one computer's page to remove it).

function named(name: string): Map<string, string> {
  return new Map<string, string>([['name', name]]);
}

/**
 * Whether the list gives way to the connect frame: until the followed computer's lists are in (the wire is up well
 * before its init), and whenever its wire failed. While the computer the list shows connects again, its lists stay
 */
export function covers(pairing: PairingModel, model: AppModel): boolean {
  if (!pairing.loaded) {
    return true;
  }
  const current = pairing.selected;
  if (current === undefined || pairing.link.kind === 'failed') {
    return true;
  }
  return !(model.computer === current.id && (model.synced || model.summaries.length > 0));
}

/** The bar over the connect frame and the list: the followed computer's name, which opens the computers page */
@ComponentV2
export struct PairBar {
  @Param title: string = '';
  @Param menu: boolean = false;
  @Event onTitle: () => void = () => {};

  build() {
    Row({ space: Metrics.s1 }) {
      Text(this.title)
        .fontSize(Type.barTitle)
        .fontWeight(FontWeight.Medium)
        .fontColor($r('app.color.fg1'))
        .maxLines(1)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
      if (this.menu) {
        Lucide({ name: 'chevronDown', tint: $r('app.color.fg3'), extent: Metrics.icon })
      }
    }
    .id('list.computer')
    .padding({ left: Metrics.s2, right: Metrics.s2, top: Metrics.s1, bottom: Metrics.s1 })
    .borderRadius(Metrics.barCtl / 2)
    .onClick(() => {
      if (this.menu) {
        this.onTitle();
      }
    })
  }
}

/**
 * The page in the list's place while it has nothing to list: no computer yet, the followed one connecting, offline,
 * removed this phone, or refused the wire. One frame shows every case, so only the sentence changes between them
 */
@ComponentV2
export struct PairConnect {
  @Require @Param pairing: PairingModel;
  /** This build carries the remote access library */
  @Param available: boolean = true;
  @Event onComputers: () => void = () => {};

  @Builder
  hero() {
    if (!this.available) {
      PairHero({ icon: 'laptop', line: t('devices.unavailable.component'), dim: true })
    } else if (this.pairing.selected === undefined) {
      PairHero({
        icon: 'laptop', line: t('mobile.pair.none'), action: t('mobile.pair.add'),
        onAction: () => {
          this.pairing.addOpen = true;
        }
      })
    } else {
      PairHero({
        icon: 'laptop',
        line: this.line(this.pairing.selected),
        spin: this.connecting(),
        dim: !this.connecting(),
        action: this.action(),
        onAction: () => {
          if (this.pairing.link.error?.kind === 'Revoked') {
            this.pairing.addOpen = true;
          } else {
            this.pairing.connectSelected();
          }
        }
      })
    }
  }

  private connecting(): boolean {
    const l = this.pairing.link;
    // Online too until the engine's lists are in; a background close dials again on the way back
    return l.kind !== 'failed' || l.error?.kind === 'Suspended';
  }

  private line(c: PairedComputer): string {
    if (this.connecting()) {
      return t('mobile.pair.connecting');
    }
    switch (this.pairing.link.error?.kind) {
      case 'Offline':
        return t('mobile.pair.offline', named(c.name));
      case 'Revoked':
        return t('mobile.pair.revoked', named(c.name));
      case 'Incompatible':
        return t('mobile.pair.incompatible', named(c.name));
      default:
        return t('mobile.pair.connectFailed', named(c.name));
    }
  }

  private action(): string | undefined {
    if (this.connecting()) {
      return undefined;
    }
    return this.pairing.link.error?.kind === 'Revoked' ? t('mobile.pair.again') : t('mobile.pair.retry');
  }

  build() {
    Column() {
      Stack() {
        PairBar({
          title: this.pairing.selected?.name ?? t('mobile.home'),
          menu: this.pairing.selected !== undefined,
          onTitle: () => this.onComputers()
        })
      }
      .height(Metrics.barCtl + Metrics.s2)
      .width('100%')
      Column() {
        if (this.pairing.loaded || !this.available) {
          PairHeroPlacement() {
            this.hero()
          }
        }
      }
      .id('pair.hero')
      .layoutWeight(1)
      .width('100%')
    }
    .backgroundColor($r('app.color.bg0'))
    .width('100%')
    .height('100%')
  }
}

/** A computer's row: its glyph with the online dot, name, how it is reached; the followed one ticked */
@ComponentV2
struct ComputerRow {
  @Require @Param pairing: PairingModel;
  @Require @Param computer: PairedComputer;
  @Event onInfo: () => void = () => {};

  build() {
    Row({ space: Metrics.gap }) {
      Stack({ alignContent: Alignment.BottomEnd }) {
        Lucide({ name: 'laptop', tint: $r('app.color.fg2'), extent: Metrics.rowMark })
        // Online: a dot on the glyph's corner, the same place on every row whatever the value's width
        if (this.pairing.online(this.computer) === true) {
          Circle()
            .width(PairMetrics.dot)
            .height(PairMetrics.dot)
            .fill($r('app.color.ok'))
            .stroke($r('app.color.form_row'))
            .strokeWidth(PairMetrics.dotCut)
            .translate({ x: PairMetrics.dot / 2, y: PairMetrics.dot / 2 })
        }
      }
      .width(Metrics.rowMark)
      .height(Metrics.rowMark)
      Text(this.computer.name)
        .fontSize(Type.text1)
        .fontColor($r('app.color.fg1'))
        .maxLines(1)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
        .layoutWeight(1)
      if (this.pairing.status(this.computer) !== undefined) {
        Text(this.pairing.status(this.computer))
          .fontSize(Type.text1)
          .fontColor($r('app.color.fg2'))
      }
      if (this.computer.id === this.pairing.selected?.id) {
        Lucide({ name: 'check', tint: $r('app.color.fg1'), extent: Metrics.icon })
      }
      Stack() {
        Lucide({ name: 'chevronRight', tint: $r('app.color.fg3'), extent: Metrics.icon })
      }
      .id(`computer.info:${this.computer.id}`)
      .width(Metrics.ctl)
      .height(Metrics.ctl)
      .onClick(() => this.onInfo())
    }
    .padding({ left: Metrics.pad, right: Metrics.s1 })
    .height(PairMetrics.formRow)
    .width('100%')
  }
}

/** Every paired computer (tap to follow it), then add another; a row's chevron opens its page */
@ComponentV2
export struct ComputersPage {
  @Require @Param pairing: PairingModel;
  @Event onBack: () => void = () => {};
  @Event onOpen: (id: string) => void = () => {};

  aboutToAppear(): void {
    this.pairing.refreshPresence();
  }

  build() {
    Column() {
      Row({ space: Metrics.s2 }) {
        Stack() {
          Lucide({ name: 'arrowLeft', tint: $r('app.color.fg1'), extent: Metrics.iconCtl })
        }
        .id('computers.back')
        .width(Metrics.ctl)
        .height(Metrics.ctl)
        .onClick(() => this.onBack())
        Text(t('mobile.pair.computers'))
          .fontSize(Type.text1)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg1'))
          .layoutWeight(1)
      }
      .padding({ left: Metrics.s1, right: Metrics.s1 })
      .height(Metrics.barCtl + Metrics.s2)
      .width('100%')

      Scroll() {
        Column() {
          Column() {
            ForEach(this.pairing.computers, (c: PairedComputer) => {
              ComputerRow({ pairing: this.pairing, computer: c, onInfo: () => this.onOpen(c.id) })
                .id(`computer:${c.id}`)
                .onClick(() => {
                  this.pairing.select(c.id);
                  this.onBack();
                })
              Divider().color($r('app.color.line')).margin({ left: Metrics.pad + Metrics.rowMark + Metrics.gap })
            }, (c: PairedComputer) => `${c.id}:${c.name}`)
            Row({ space: Metrics.gap }) {
              Lucide({ name: 'plus', tint: $r('app.color.fg2'), extent: Metrics.rowMark })
              Text(t('mobile.pair.add')).fontSize(Type.text1).fontColor($r('app.color.fg1')).layoutWeight(1)
            }
            .id('computers.add')
            .padding({ left: Metrics.pad, right: Metrics.pad })
            .height(PairMetrics.formRow)
            .width('100%')
            .onClick(() => {
              this.pairing.addOpen = true;
            })
          }
          .borderRadius(Metrics.rLg)
          .backgroundColor($r('app.color.form_row'))
          Text(t('mobile.pair.computers.desc'))
            .fontSize(Type.text4)
            .fontColor($r('app.color.fg2'))
            .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2 })
            .width('100%')
        }
        .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2, bottom: Metrics.page })
        .constraintSize({ maxWidth: Metrics.contentW })
        .width('100%')
      }
      .scrollBar(BarState.Off)
      .align(Alignment.Top)
      .layoutWeight(1)
    }
    .backgroundColor($r('app.color.form_page'))
    .width('100%')
    .height('100%')
  }
}

/** One paired computer: how it is reached and since when, and removing it */
@ComponentV2
export struct ComputerPage {
  @Require @Param pairing: PairingModel;
  @Param computerId: string = '';
  @Event onBack: () => void = () => {};

  aboutToAppear(): void {
    this.pairing.refreshPresence();
  }

  private computer(): PairedComputer | undefined {
    return this.pairing.computers.find((c: PairedComputer) => c.id === this.computerId);
  }

  private confirmRemove(c: PairedComputer): void {
    this.getUIContext().getPromptAction().showDialog({
      title: t('mobile.pair.remove.confirm', named(c.name)),
      buttons: [
        { text: t('mobile.cancel'), color: $r('app.color.fg1') },
        { text: t('common.remove'), color: $r('app.color.danger') }
      ]
    }).then((r: promptAction.ShowDialogSuccessResponse) => {
      if (r.index === 1) {
        this.pairing.remove(c.id);
        this.onBack();
      }
    }).catch(() => {});
  }

  @Builder
  row(label: string, value: string | undefined) {
    Row({ space: Metrics.gap }) {
      Text(label).fontSize(Type.text1).fontColor($r('app.color.fg1')).layoutWeight(1)
      if (value !== undefined) {
        Text(value).fontSize(Type.text1).fontColor($r('app.color.fg2'))
      } else {
        LoadingProgress().color($r('app.color.fg2')).width(Metrics.icon).height(Metrics.icon)
      }
    }
    .padding({ left: Metrics.pad, right: Metrics.pad })
    .height(PairMetrics.formRow)
    .width('100%')
  }

  build() {
    Column() {
      Row({ space: Metrics.s2 }) {
        Stack() {
          Lucide({ name: 'arrowLeft', tint: $r('app.color.fg1'), extent: Metrics.iconCtl })
        }
        .id('computer.back')
        .width(Metrics.ctl)
        .height(Metrics.ctl)
        .onClick(() => this.onBack())
        Text(this.computer()?.name ?? '')
          .fontSize(Type.text1)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg1'))
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
          .layoutWeight(1)
      }
      .padding({ left: Metrics.s1, right: Metrics.s1 })
      .height(Metrics.barCtl + Metrics.s2)
      .width('100%')

      if (this.computer() !== undefined) {
        Column({ space: Metrics.s6 }) {
          Column() {
            this.row(t('mobile.pair.connection'), this.pairing.status(this.computer() as PairedComputer))
            Divider().color($r('app.color.line')).margin({ left: Metrics.pad })
            this.row(t('mobile.pair.pairedAt'), new Date((this.computer() as PairedComputer).pairedAt).toLocaleDateString())
          }
          .borderRadius(Metrics.rLg)
          .backgroundColor($r('app.color.form_row'))
          Column() {
            Row() {
              Text(t('mobile.pair.remove')).fontSize(Type.text1).fontColor($r('app.color.danger'))
            }
            .id('computer.remove')
            .padding({ left: Metrics.pad, right: Metrics.pad })
            .height(PairMetrics.formRow)
            .width('100%')
            .borderRadius(Metrics.rLg)
            .backgroundColor($r('app.color.form_row'))
            .onClick(() => this.confirmRemove(this.computer() as PairedComputer))
            Text(t('mobile.pair.remove.desc'))
              .fontSize(Type.text4)
              .fontColor($r('app.color.fg2'))
              .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2 })
              .width('100%')
          }
        }
        .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2 })
        .constraintSize({ maxWidth: Metrics.contentW })
        .width('100%')
      }
    }
    .backgroundColor($r('app.color.form_page'))
    .width('100%')
    .height('100%')
  }
}
