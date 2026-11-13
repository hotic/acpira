import { pasteboard } from '@kit.BasicServicesKit';
import { Metrics, PairMetrics, PairType, Type } from '../design/Tokens';
import { Lucide } from '../components/Glyphs';
import { PairHero, PairHeroPlacement } from '../components/PairHero';
import { PairCameraCard } from '../components/PairScanner';
import { t } from '../store/L10n';
import { PairingModel } from '../store/PairingModel';
import { NearbyBrowser } from '../remote/Network';
import { isPairingLink, PairingError } from '../remote/PairingService';
import { NearbyComputer, PairedComputer } from '../remote/Stores';
import { DEV_NEARBY } from '../generated/Assets.generated';

// The add-computer sheet (apps/ios Pairing/PairSheet.swift + PairCodePage.swift, apps/android ui/PairSheet.kt). Its form
// leads with the camera card, then the computers this network shows (picking one asks for the code on its card) and a
// pairing link, pasted or typed. A code scanned or a link entered pairs at once; a link opened from outside the app asks
// first. Pairing, connecting, connected and a refusal share one connect frame that replaces the form in place, and the
// sheet goes once the new computer answers.

const CODE_LENGTH = 6;

/** What the sheet shows: `choose` (the form), `code` (a nearby computer's code), `confirm` / `pairing` (a link), `joined`
 * (paired, waiting for its wire) or `failed` (with the way on: the same link again, else back to the form) */
class PairPhase {
  kind: string = 'choose';
  link?: string;
  name?: string;
  nearby?: NearbyComputer;
  computer?: PairedComputer;
  message?: string;
  retryLink?: string;
}

function phase(kind: string): PairPhase {
  const p = new PairPhase();
  p.kind = kind;
  return p;
}

function named(name: string): Map<string, string> {
  return new Map<string, string>([['name', name]]);
}

/** Why a scanned or opened link did not pair: a stale code asks for the new one the computer's card shows by now */
function linkMessage(e: PairingError, name: string | undefined): string {
  switch (e.kind) {
    case 'Expired':
      return t('mobile.pair.scan.expired');
    case 'Used':
      return t('mobile.pair.scan.used');
    case 'Impostor':
      return t('mobile.pair.impostor');
    case 'InvalidLink':
      return t('mobile.pair.invalidLink');
    default:
      break;
  }
  if (name !== undefined) {
    if (e.kind === 'Closed') {
      return t('mobile.pair.closed', named(name));
    }
    if (e.kind === 'Unreachable') {
      return t('mobile.pair.unreachable', named(name));
    }
    // No relay on one side: only the same Wi-Fi joins the two now
    if (e.kind === 'NoRelay') {
      return t('mobile.pair.noRelay', named(name));
    }
  }
  return t('mobile.pair.failed');
}

/** Why a code was refused, in the code page's own words */
function codeMessage(e: PairingError, name: string): string {
  switch (e.kind) {
    case 'Wrong':
      return t('mobile.pair.code.wrong');
    case 'Expired':
      return t('mobile.pair.code.expired');
    case 'Used':
      return t('mobile.pair.code.used');
    case 'Closed':
      return t('mobile.pair.closed', named(name));
    // A code pairs over this network only, so no relay never stands in its way
    case 'Unreachable':
    case 'NoRelay':
      return t('mobile.pair.unreachable', named(name));
    case 'Impostor':
      return t('mobile.pair.impostor');
    default:
      return t('mobile.pair.failed');
  }
}

function asPairingError(e: Object): PairingError {
  return e instanceof PairingError ? e : new PairingError('Other', `${e}`);
}

/** The six digits a nearby computer's card shows: one hidden field behind six boxes, the sixth digit sends the code */
@ComponentV2
struct PairCodeEntry {
  @Require @Param computer: NearbyComputer;
  @Param checking: boolean = false;
  @Param error: string | undefined = undefined;
  @Event onCode: (digits: string) => void = () => {};
  @Local code: string = '';
  @Local focused: boolean = false;
  @Local caretOn: boolean = true;
  private blink: number = -1;

  aboutToAppear(): void {
    this.blink = setInterval(() => {
      this.caretOn = !this.caretOn;
    }, PairMetrics.blinkMs);
  }

  aboutToDisappear(): void {
    clearInterval(this.blink);
  }

  /** A refused code clears the boxes */
  @Monitor('error')
  refused(): void {
    if (this.error !== undefined) {
      this.code = '';
    }
  }

  @Builder
  box(i: number) {
    Stack() {
      if (i < this.code.length) {
        Text(this.code.charAt(i))
          .fontSize(PairType.digit)
          .fontWeight(FontWeight.Medium)
          .fontColor(this.checking ? $r('app.color.fg2') : $r('app.color.fg_strong'))
      } else if (this.focused && !this.checking && i === this.code.length) {
        Column()
          .width(PairMetrics.caret)
          .height(PairMetrics.caretH)
          .backgroundColor($r('app.color.fg1'))
          .opacity(this.caretOn ? 1 : 0)
      }
    }
    .width(PairMetrics.codeBox)
    // A window narrower than the six boxes narrows them all alike
    .flexShrink(1)
    .height(PairMetrics.codeBoxH)
    .borderRadius(Metrics.rLg)
    .backgroundColor($r('app.color.form_row'))
    .border({
      width: PairMetrics.codeLine,
      color: this.focused && !this.checking && i === this.code.length ? $r('app.color.fg1') : $r('app.color.line')
    })
    .margin({ left: i === CODE_LENGTH / 2 ? PairMetrics.codeSplit - Metrics.s2 : 0 })
  }

  build() {
    // Scrolls, so a short window with the keyboard up (a phone on its side) still shows the boxes
    Scroll() {
      Column() {
        Text(t('mobile.pair.code.hint'))
          .fontSize(Type.body)
          .lineHeight(Type.bodyLine)
          .fontColor($r('app.color.fg2'))
          .textAlign(TextAlign.Center)
          .margin({ bottom: Metrics.page })
        Stack() {
          // Kept all but invisible rather than hidden: a hidden field takes no focus
          TextInput({ text: this.code })
            .id('pair.code.field')
            .type(InputType.Number)
            .maxLength(CODE_LENGTH)
            .caretColor(Color.Transparent)
            .fontColor(Color.Transparent)
            .backgroundColor(Color.Transparent)
            .width('100%')
            .height(PairMetrics.codeBoxH)
            // defaultFocus only counts when a page is first created; this page swaps in inside a sheet or a step
            .onAppear(() => {
              setTimeout(() => {
                focusControl.requestFocus('pair.code.field');
              }, 0);
            })
            .onFocus(() => {
              this.focused = true;
            })
            .onBlur(() => {
              this.focused = false;
            })
            .onChange((typed: string) => {
              // What is typed while a code is checked is dropped
              if (this.checking) {
                return;
              }
              const digits = typed.replace(/[^0-9]/g, '').slice(0, CODE_LENGTH);
              this.code = digits;
              if (digits.length === CODE_LENGTH) {
                this.onCode(digits);
              }
            })
          Row({ space: Metrics.s2 }) {
            ForEach([0, 1, 2, 3, 4, 5], (i: number) => {
              this.box(i)
            }, (i: number) => `${i}`)
          }
          .justifyContent(FlexAlign.Center)
          .width('100%')
          .hitTestBehavior(HitTestMode.None)
        }
        // Checking, or why the last code was refused; the slot stays, so nothing under it moves
        Stack({ alignContent: Alignment.Top }) {
          if (this.checking) {
            LoadingProgress().color($r('app.color.fg2')).width(Metrics.iconCtl).height(Metrics.iconCtl)
          } else if (this.error !== undefined) {
            Text(this.error)
              .id('pair.code.error')
              .fontSize(Type.text3)
              .fontColor($r('app.color.danger'))
              .textAlign(TextAlign.Center)
          }
        }
        .width('100%')
        .constraintSize({ minHeight: PairMetrics.statusSlot })
        .margin({ top: Metrics.gap })
      }
      .padding({ top: Metrics.page * 2, left: Metrics.pad, right: Metrics.pad })
      .width('100%')
    }
    .align(Alignment.Top)
    .scrollBar(BarState.Off)
    .edgeEffect(EdgeEffect.None)
    .width('100%')
  }
}

@ComponentV2
export struct PairSheet {
  @Require @Param pairing: PairingModel;
  /** The first run's step (pages/Welcome.ets) rather than a sheet: back instead of close, then the fallbacks */
  @Param inline: boolean = false;
  /** The phase to open on (screenshots): `code` opens the first nearby computer's code page */
  @Param start: string = 'choose';
  /** Paired (its wire up or failed), or closed */
  @Event onClose: () => void = () => {};
  /** Inline: back to the step before */
  @Event onBack: () => void = () => {};
  /** Inline: an engine by address and token instead (the development engine, apps/ios/scripts/engine.sh) */
  @Event onAddress: () => void = () => {};
  /** Inline: on with the demo */
  @Event onLater: () => void = () => {};
  @Local phase: PairPhase = phase('choose');
  @Local nearby: NearbyComputer[] = [];
  @Local notice: string | undefined = undefined;
  @Local typed: string = '';
  @Local checking: boolean = false;
  @Local codeError: string | undefined = undefined;
  /** The form's height, for the camera card (components/PairScanner.ets) */
  @Local formRoom: number = 0;
  private browser: NearbyBrowser | undefined = undefined;
  private noticeGen: number = 0;
  private closeTimer: number = -1;
  /** When the computer answered the pairing */
  private pairedAt: number = 0;

  aboutToAppear(): void {
    // Browsing runs for the sheet's whole life, so coming back from the code page keeps the rows where they were
    const pinned = this.pairing.pinnedNearby ?? NearbyBrowser.pinned(DEV_NEARBY);
    this.nearby = pinned !== undefined ? [pinned] : [];
    this.browser = this.pairing.service.nearby((found: NearbyComputer[]) => {
      this.nearby = pinned !== undefined ? [pinned].concat(found.filter((c: NearbyComputer) => c.id !== pinned.id)) : found;
    });
    this.browser.start();
    if (this.start === 'code' && this.nearby.length > 0) {
      const p = phase('code');
      p.nearby = this.nearby[0];
      this.phase = p;
    }
    this.takePending();
  }

  aboutToDisappear(): void {
    this.browser?.stop();
    clearTimeout(this.closeTimer);
  }

  @Monitor('pairing.pendingLink')
  pendingChanged(): void {
    this.takePending();
  }

  /**
   * Connected: the sheet goes once the paired moment has held as long as the computer's card holds it, and a beat after
   * a wire that came up later. A wire that fails right after pairing leaves the sheet too, and the list says why
   */
  @Monitor('pairing.link')
  wireChanged(): void {
    const p = this.phase;
    if (p.kind !== 'joined' || p.computer === undefined || this.pairing.selected?.id !== p.computer.id) {
      return;
    }
    const link = this.pairing.link;
    clearTimeout(this.closeTimer);
    if (link.kind === 'online') {
      const held = Date.now() - this.pairedAt;
      this.closeTimer = setTimeout(() => this.onClose(), Math.max(PairMetrics.holdMs - held, PairMetrics.doneMs));
    } else if (link.kind === 'failed') {
      this.onClose();
    }
  }

  /** A link opened from outside while the sheet is up, or the one that raised it */
  private takePending(): void {
    const link = this.pairing.pendingLink;
    if (link === undefined) {
      return;
    }
    this.pairing.pendingLink = undefined;
    try {
      const p = phase('confirm');
      p.link = link;
      p.name = this.pairing.service.computerName(link);
      this.phase = p;
    } catch (e) {
      this.fail(undefined, linkMessage(asPairingError(e as Object), undefined), undefined);
    }
  }

  private fail(name: string | undefined, message: string, retryLink: string | undefined): void {
    const p = phase('failed');
    p.name = name;
    p.message = message;
    p.retryLink = retryLink;
    this.phase = p;
  }

  private joined(computer: PairedComputer): void {
    const p = phase('joined');
    p.computer = computer;
    p.name = computer.name;
    this.pairedAt = Date.now();
    this.phase = p;
    this.pairing.paired(computer);
  }

  /** Pairs through a scanned, pasted, typed or confirmed link */
  private pair(link: string): void {
    let name: string;
    try {
      name = this.pairing.service.computerName(link);
    } catch (e) {
      this.fail(undefined, linkMessage(asPairingError(e as Object), undefined), undefined);
      return;
    }
    const p = phase('pairing');
    p.link = link;
    p.name = name;
    this.phase = p;
    this.pairing.service.pair(link).then((computer: PairedComputer) => {
      if (this.phase !== p) {
        return;
      }
      this.joined(computer);
    }).catch((e: Object) => {
      if (this.phase !== p) {
        return;
      }
      const error = asPairingError(e);
      // Not answered: the same link may work once the computer or this network comes back
      const unanswered = error.kind === 'Unreachable' || error.kind === 'NoRelay';
      this.fail(name, linkMessage(error, name), unanswered ? link : undefined);
    });
  }

  private submitCode(computer: NearbyComputer, digits: string): void {
    if (this.checking) {
      return;
    }
    this.checking = true;
    this.codeError = undefined;
    this.pairing.service.pairCode(computer, digits).then((paired: PairedComputer) => {
      this.checking = false;
      this.joined(paired);
    }).catch((e: Object) => {
      this.checking = false;
      this.codeError = codeMessage(asPairingError(e), computer.name);
    });
  }

  private entered(text: string): void {
    const link = text.trim();
    if (isPairingLink(link)) {
      this.typed = '';
      this.pair(link);
      return;
    }
    // Not a pairing link: the row says so for a moment, in place
    const gen = ++this.noticeGen;
    this.notice = t('mobile.pair.paste.notLink');
    setTimeout(() => {
      if (gen === this.noticeGen) {
        this.notice = undefined;
      }
    }, PairMetrics.noticeMs);
  }

  private act(): void {
    const p = this.phase;
    if (p.kind === 'confirm' && p.link !== undefined) {
      this.pair(p.link);
    } else if (p.kind === 'failed') {
      if (p.retryLink !== undefined) {
        this.pair(p.retryLink);
      } else {
        this.phase = phase('choose');
      }
    }
  }

  private title(): string {
    switch (this.phase.kind) {
      case 'choose':
        return this.inline ? t('mobile.pair.connectTitle') : t('mobile.pair.add');
      case 'code':
        return t('mobile.pair.code.title');
      default:
        return '';
    }
  }

  @Builder
  bar() {
    Row({ space: Metrics.s2 }) {
      Stack() {
        if (this.phase.kind === 'code' || (this.phase.kind === 'failed' && this.phase.retryLink === undefined) ||
          (this.inline && this.phase.kind === 'choose')) {
          Lucide({ name: 'arrowLeft', tint: $r('app.color.fg1'), extent: Metrics.iconCtl })
        }
      }
      .id('pair.back')
      .width(Metrics.ctl)
      .height(Metrics.ctl)
      .onClick(() => {
        if (this.phase.kind === 'code' || this.phase.kind === 'failed') {
          this.codeError = undefined;
          this.phase = phase('choose');
        } else if (this.inline && this.phase.kind === 'choose') {
          this.onBack();
        }
      })
      Column() {
        Text(this.title())
          .fontSize(Type.text1)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg1'))
          .maxLines(1)
        if (this.phase.kind === 'code' && this.phase.nearby !== undefined) {
          Text(this.phase.nearby.name)
            .fontSize(Type.text4)
            .fontColor($r('app.color.fg2'))
            .maxLines(1)
        }
      }
      .layoutWeight(1)
      if (!this.inline) {
        Stack() {
          Lucide({ name: 'x', tint: $r('app.color.fg1'), extent: Metrics.iconCtl })
        }
        .id('pair.close')
        .width(Metrics.ctl)
        .height(Metrics.ctl)
        .borderRadius(Metrics.ctl / 2)
        .backgroundColor($r('app.color.active'))
        .onClick(() => this.onClose())
      }
    }
    .padding({ left: Metrics.s2, right: Metrics.pad })
    .height(Metrics.barCtl + Metrics.s2)
    .width('100%')
  }

  @Builder
  sectionHeader(title: string, busy: boolean) {
    Row({ space: Metrics.s2 }) {
      Text(title).fontSize(Type.text4).fontColor($r('app.color.fg2'))
      if (busy) {
        LoadingProgress().color($r('app.color.fg3')).width(Metrics.icon).height(Metrics.icon)
      }
    }
    .padding({ left: Metrics.pad, top: PairMetrics.sectionGap - Metrics.s2, bottom: Metrics.s2 })
    .width('100%')
  }

  @Builder
  form() {
    Scroll() {
      Column() {
        PairCameraCard({ active: this.phase.kind === 'choose', room: this.formRoom, onLink: (link: string) => this.pair(link) })
          .borderRadius(Metrics.rLg)
          .clip(true)

        this.sectionHeader(t('mobile.pair.nearby'), true)
        Column() {
          if (this.nearby.length === 0) {
            Text(t('mobile.pair.nearby.none'))
              .fontSize(Type.text3)
              .fontColor($r('app.color.fg3'))
              .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.gap, bottom: Metrics.gap })
              .width('100%')
          }
          ForEach(this.nearby, (c: NearbyComputer, i: number) => {
            Row({ space: Metrics.gap }) {
              Lucide({ name: 'laptop', tint: $r('app.color.fg2'), extent: Metrics.rowMark })
              Text(c.name).fontSize(Type.text1).fontColor($r('app.color.fg1')).maxLines(1).layoutWeight(1)
              Lucide({ name: 'chevronRight', tint: $r('app.color.fg3'), extent: Metrics.icon })
            }
            .id(`pair.nearby:${c.id}`)
            .padding({ left: Metrics.pad, right: Metrics.pad })
            .height(PairMetrics.formRow)
            .width('100%')
            .onClick(() => {
              const p = phase('code');
              p.nearby = c;
              this.codeError = undefined;
              this.phase = p;
            })
            if (i < this.nearby.length - 1) {
              Divider().color($r('app.color.line')).margin({ left: Metrics.pad + Metrics.rowMark + Metrics.gap })
            }
          }, (c: NearbyComputer) => `${c.id}:${c.name}`)
        }
        .borderRadius(Metrics.rLg)
        .backgroundColor($r('app.color.form_row'))

        Row({ space: Metrics.gap }) {
          Lucide({ name: 'link', tint: $r('app.color.fg2'), extent: Metrics.rowMark })
          TextInput({ placeholder: this.notice ?? t('mobile.pair.paste'), text: this.typed })
            .id('pair.link')
            .fontSize(Type.text1)
            .placeholderColor(this.notice === undefined ? $r('app.color.fg1') : $r('app.color.fg3'))
            .placeholderFont({ size: Type.text1 })
            .backgroundColor(Color.Transparent)
            .padding(0)
            .enterKeyType(EnterKeyType.Go)
            .layoutWeight(1)
            .onChange((v: string) => {
              this.typed = v;
            })
            .onSubmit(() => this.entered(this.typed))
          // The system's paste control reads the clipboard on the tap without a permission prompt
          PasteButton({ text: PasteDescription.PASTE, buttonType: ButtonType.Capsule })
            .id('pair.paste')
            .fontSize(Type.text4)
            .fontColor($r('app.color.fg1'))
            .backgroundColor($r('app.color.form_page'))
            .onClick((event: ClickEvent, result: PasteButtonOnClickResult) => {
              if (result !== PasteButtonOnClickResult.SUCCESS) {
                return;
              }
              try {
                this.entered(pasteboard.getSystemPasteboard().getDataSync().getPrimaryText() ?? '');
              } catch (e) {
                this.entered('');
              }
            })
        }
        .padding({ left: Metrics.pad, right: Metrics.gap })
        .height(PairMetrics.formRow)
        .width('100%')
        .borderRadius(Metrics.rLg)
        .backgroundColor($r('app.color.form_row'))
        .margin({ top: PairMetrics.sectionGap })

        if (this.inline) {
          // The first run's fallbacks: an engine by address (no pairing on the computer's side), or the demo for now
          Row({ space: Metrics.gap }) {
            Lucide({ name: 'globe', tint: $r('app.color.fg2'), extent: Metrics.rowMark })
            Text(t('connections.address')).fontSize(Type.text1).fontColor($r('app.color.fg1')).layoutWeight(1)
            Lucide({ name: 'chevronRight', tint: $r('app.color.fg3'), extent: Metrics.icon })
          }
          .id('pair.address')
          .padding({ left: Metrics.pad, right: Metrics.pad })
          .height(PairMetrics.formRow)
          .width('100%')
          .borderRadius(Metrics.rLg)
          .backgroundColor($r('app.color.form_row'))
          .margin({ top: Metrics.gap })
          .onClick(() => this.onAddress())
          Text(t('mobile.welcome.later'))
            .id('pair.later')
            .fontSize(Type.text2)
            .fontColor($r('app.color.fg3'))
            .padding({ top: Metrics.s3, bottom: Metrics.s3 })
            .margin({ top: PairMetrics.sectionGap })
            .onClick(() => this.onLater())
        }
      }
      .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2, bottom: Metrics.page })
      .constraintSize({ maxWidth: Metrics.contentW })
      .width('100%')
    }
    .scrollBar(BarState.Off)
    .align(Alignment.Top)
    .layoutWeight(1)
    .width('100%')
    .onSizeChange((_: SizeOptions, now: SizeOptions) => {
      this.formRoom = Number(now.height);
    })
  }

  @Builder
  stage() {
    PairHeroPlacement() {
      PairHero({
        icon: 'laptop',
        name: this.phase.name,
        line: this.stageLine(),
        spin: this.phase.kind === 'pairing' || this.phase.kind === 'joined',
        paired: this.phase.kind === 'joined',
        path: this.stagePath(),
        action: this.stageAction(),
        secondary: this.phase.kind === 'failed' && this.phase.retryLink !== undefined ? t('mobile.pair.scan.again') : undefined,
        dim: this.phase.kind === 'failed',
        onAction: () => this.act(),
        onSecondary: () => {
          this.phase = phase('choose');
        }
      })
    }
  }

  private stageLine(): string {
    switch (this.phase.kind) {
      case 'confirm':
        return t('mobile.pair.confirm');
      case 'pairing':
      case 'joined':
        return t('mobile.pair.connecting');
      case 'failed':
        return this.phase.message ?? t('mobile.pair.failed');
      default:
        return '';
    }
  }

  /** The newly paired computer's path once its wire is up; it follows the wire */
  private stagePath(): string | undefined {
    const p = this.phase;
    if (p.kind !== 'joined' || p.computer === undefined || this.pairing.selected?.id !== p.computer.id) {
      return undefined;
    }
    return this.pairing.link.kind === 'online' ? this.pairing.link.path : undefined;
  }

  private stageAction(): string | undefined {
    const p = this.phase;
    if (p.kind === 'confirm') {
      return t('mobile.pair.connect');
    }
    if (p.kind === 'failed') {
      return p.retryLink !== undefined ? t('mobile.pair.retry') : t('mobile.pair.scan.again');
    }
    return undefined;
  }

  build() {
    Column() {
      this.bar()
      if (this.phase.kind === 'choose') {
        this.form()
      } else if (this.phase.kind === 'code' && this.phase.nearby !== undefined) {
        PairCodeEntry({
          computer: this.phase.nearby,
          checking: this.checking,
          error: this.codeError,
          onCode: (digits: string) => {
            const c = this.phase.nearby;
            if (c !== undefined) {
              this.submitCode(c, digits);
            }
          }
        })
          .layoutWeight(1)
      } else {
        Column() {
          this.stage()
        }
        .layoutWeight(1)
        .width('100%')
      }
    }
    .backgroundColor($r('app.color.form_page'))
    .width('100%')
    .height('100%')
  }
}
