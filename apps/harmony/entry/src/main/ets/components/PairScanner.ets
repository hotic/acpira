import { abilityAccessCtrl, common, Permissions } from '@kit.AbilityKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { camera } from '@kit.CameraKit';
import { image } from '@kit.ImageKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { Metrics, PairMetrics, Type } from '../design/Tokens';
import { t } from '../store/L10n';
import { net } from '../remote/Net';
import { isPairingLink } from '../remote/PairingService';
import { Lucide } from './Glyphs';

// The camera card that leads the add-computer sheet (apps/ios Pairing/PairScanner.swift, apps/android
// ui/PairScanner.kt): the back camera under a viewfinder, its frames read for QR codes. The public OpenHarmony SDK has
// no barcode scanner (ScanKit is HarmonyOS-only), so the frames' luma goes to the pairing library's decoder (rqrr in
// rust/crates/acpira-mobile-ohos). Where there is no camera or no library the card is a quiet stand-in; where access was
// refused it says so. Any QR code that is not an `acpira://pair/…` link is ignored.

const DOMAIN = 0xacb1;
const TAG = 'AcpiraPair';
const CAMERA: Permissions = 'ohos.permission.CAMERA';
/** The smallest preview size worth decoding, and how often a frame is read */
const DECODE_MIN_W = 640;
const DECODE_EVERY_MS = 250;
const RECEIVER_CAPACITY = 4;

enum Source {
  Unknown,
  Live,
  Unavailable,
  Denied
}

/** The four rounded corners of the viewfinder, each one box with two of its borders drawn */
@ComponentV2
struct FinderCorners {
  @Param ink: ResourceColor = $r('app.color.pair_check_ink');

  build() {
    Stack() {
      Column().width(PairMetrics.finderCorner).height(PairMetrics.finderCorner)
        .border({ width: { top: PairMetrics.finderStroke, left: PairMetrics.finderStroke }, color: this.ink,
          radius: { topLeft: PairMetrics.finderCorner / 2 } })
        .position({ left: 0, top: 0 })
      Column().width(PairMetrics.finderCorner).height(PairMetrics.finderCorner)
        .border({ width: { top: PairMetrics.finderStroke, right: PairMetrics.finderStroke }, color: this.ink,
          radius: { topRight: PairMetrics.finderCorner / 2 } })
        .position({ right: 0, top: 0 })
      Column().width(PairMetrics.finderCorner).height(PairMetrics.finderCorner)
        .border({ width: { bottom: PairMetrics.finderStroke, right: PairMetrics.finderStroke }, color: this.ink,
          radius: { bottomRight: PairMetrics.finderCorner / 2 } })
        .position({ right: 0, bottom: 0 })
      Column().width(PairMetrics.finderCorner).height(PairMetrics.finderCorner)
        .border({ width: { bottom: PairMetrics.finderStroke, left: PairMetrics.finderStroke }, color: this.ink,
          radius: { bottomLeft: PairMetrics.finderCorner / 2 } })
        .position({ left: 0, bottom: 0 })
    }
    .width(PairMetrics.finder)
    .height(PairMetrics.finder)
  }
}

@ComponentV2
export struct PairCameraCard {
  /** Scans only while the sheet's form shows and nothing is being paired */
  @Param active: boolean = true;
  /** The height of the form the card leads (0 until measured): the card never takes all of it */
  @Param room: number = 0;
  @Event onLink: (link: string) => void = () => {};
  @Local source: Source = Source.Unknown;
  private xc: XComponentController = new XComponentController();
  private session: camera.PhotoSession | undefined = undefined;
  private input: camera.CameraInput | undefined = undefined;
  private receiver: image.ImageReceiver | undefined = undefined;
  private lastDecode: number = 0;
  private decoding: boolean = false;
  /** The preview frames' size */
  private frameW: number = 0;
  private frameH: number = 0;
  private surface: string | undefined = undefined;
  /** Bumped by every stop: a start still awaiting the camera finds it moved on and lets go of what it opened */
  private run: number = 0;

  aboutToAppear(): void {
    this.resolve();
  }

  aboutToDisappear(): void {
    this.stopCamera();
  }

  @Monitor('active')
  activeChanged(): void {
    if (!this.active) {
      this.stopCamera();
    } else if (this.source === Source.Live && this.surface !== undefined) {
      this.startCamera(this.surface);
    }
  }

  private async resolve(): Promise<void> {
    const context = this.getUIContext().getHostContext() as common.UIAbilityContext | undefined;
    if (net() === undefined || context === undefined) {
      this.source = Source.Unavailable;
      return;
    }
    // A 2in1 may lack the camera; the inner check is what the SysCap check reads
    if (!canIUse('SystemCapability.Multimedia.Camera.Core')) {
      this.source = Source.Unavailable;
      return;
    }
    try {
      if (canIUse('SystemCapability.Multimedia.Camera.Core')) {
        if (camera.getCameraManager(context).getSupportedCameras().length === 0) {
          this.source = Source.Unavailable;
          return;
        }
      }
      const at = abilityAccessCtrl.createAtManager();
      const granted = await at.requestPermissionsFromUser(context, [CAMERA]);
      this.source = granted.authResults[0] === 0 ? Source.Live : Source.Denied;
    } catch (e) {
      hilog.warn(DOMAIN, TAG, 'camera unavailable: %{public}s', `${e}`);
      this.source = Source.Unavailable;
    }
  }

  private async startCamera(surface: string): Promise<void> {
    // A start under way holds the input already
    if (this.session !== undefined || this.input !== undefined || !this.active) {
      return;
    }
    const context = this.getUIContext().getHostContext();
    if (context === undefined) {
      return;
    }
    // Every camera call sits under the capability check (a 2in1 may have none), as the SysCap check wants
    if (canIUse('SystemCapability.Multimedia.Camera.Core')) {
      try {
        const manager = camera.getCameraManager(context);
        const cameras = manager.getSupportedCameras();
        const device = cameras.find((c: camera.CameraDevice) => c.cameraPosition === camera.CameraPosition.CAMERA_POSITION_BACK)
          ?? cameras[0];
        const caps = manager.getSupportedOutputCapability(device, camera.SceneMode.NORMAL_PHOTO);
        // The smallest preview at least DECODE_MIN_W wide: enough for a code held up to it, cheap to read
        const profiles = caps.previewProfiles.slice().sort((a: camera.Profile, b: camera.Profile) => a.size.width - b.size.width);
        const profile = profiles.find((p: camera.Profile) => p.size.width >= DECODE_MIN_W) ?? profiles[profiles.length - 1];
        this.frameW = profile.size.width;
        this.frameH = profile.size.height;
        const receiver = image.createImageReceiver(profile.size, image.ImageFormat.JPEG, RECEIVER_CAPACITY);
        receiver.on('imageArrival', () => this.frame(receiver));
        this.receiver = receiver;
        // Held before the first await, so a stop meanwhile (the sheet closing) releases it; each await then checks
        const run = this.run;
        const input = manager.createCameraInput(device);
        this.input = input;
        await input.open();
        if (run !== this.run) {
          // The stop closed it before it was open
          input.close().catch((e: BusinessError) => {});
          return;
        }
        const receiving = await receiver.getReceivingSurfaceId();
        if (run !== this.run) {
          return;
        }
        const session = manager.createSession(camera.SceneMode.NORMAL_PHOTO) as camera.PhotoSession;
        this.session = session;
        session.beginConfig();
        session.addInput(input);
        session.addOutput(manager.createPreviewOutput(profile, surface));
        session.addOutput(manager.createPreviewOutput(profile, receiving));
        await session.commitConfig();
        if (run !== this.run) {
          return;
        }
        await session.start();
        if (run !== this.run) {
          // The stop ran while it started: stop it again now that it runs
          session.stop().catch((e: BusinessError) => {}).then(() => session.release()).catch((e: BusinessError) => {});
        }
      } catch (e) {
        hilog.warn(DOMAIN, TAG, 'camera start failed: %{public}s', `${e}`);
        this.stopCamera();
        this.source = Source.Unavailable;
      }
    }
  }

  private stopCamera(): void {
    this.run++;
    const session = this.session;
    this.session = undefined;
    const input = this.input;
    this.input = undefined;
    if (canIUse('SystemCapability.Multimedia.Camera.Core')) {
      // A session stopped mid-start may refuse stop(): it is released either way
      session?.stop().catch((e: BusinessError) => {}).then(() => session.release()).catch((e: BusinessError) => {});
      input?.close().catch((e: BusinessError) => {});
    }
    this.receiver?.release().catch((e: BusinessError) => {});
    this.receiver = undefined;
  }

  /** One frame: its luma read for a code now and then, every frame released */
  private frame(receiver: image.ImageReceiver): void {
    receiver.readNextImage().then((img: image.Image) => {
      const now = Date.now();
      if (this.decoding || now - this.lastDecode < DECODE_EVERY_MS || !this.active) {
        img.release();
        return;
      }
      this.decoding = true;
      this.lastDecode = now;
      let released = false;
      img.getComponent(image.ComponentType.JPEG).then((c: image.Component) => {
        // A preview frame is NV21 behind the JPEG component: its first plane is the luma, `rowStride` bytes a row. The
        // library copies it before answering and decodes off the UI thread
        const decoded = net()?.decodeQr(new Uint8Array(c.byteBuffer), this.frameW, this.frameH,
          c.rowStride > 0 ? c.rowStride : this.frameW) ?? Promise.resolve<string | null>(null);
        released = true;
        img.release();
        return decoded;
      }).then((text: string | null) => {
        this.decoding = false;
        if (text !== null && isPairingLink(text) && this.active) {
          this.onLink(text.trim());
        }
      }).catch(() => {
        if (!released) {
          img.release();
        }
        this.decoding = false;
      });
    }).catch(() => {});
  }

  /** Gives way where the form is short (a phone on its side), down to the viewfinder and its margin */
  private cardHeight(): number {
    if (this.room <= 0) {
      return PairMetrics.scanCard;
    }
    return Math.min(PairMetrics.scanCard, Math.max(PairMetrics.finder + Metrics.page * 2, this.room - PairMetrics.scanPeek));
  }

  build() {
    Stack() {
      if (this.source === Source.Live) {
        XComponent({ type: XComponentType.SURFACE, controller: this.xc })
          .onLoad(() => {
            this.surface = this.xc.getXComponentSurfaceId();
            this.startCamera(this.surface);
          })
          .width('100%')
          .height('100%')
        Column().width('100%').height('100%').backgroundColor($r('app.color.pair_finder_dim'))
        FinderCorners({ ink: $r('app.color.pair_check_ink') })
      } else if (this.source === Source.Denied) {
        Column({ space: Metrics.gap }) {
          Lucide({ name: 'camera', tint: $r('app.color.fg3'), extent: PairMetrics.glyph })
          Text(t('mobile.pair.camera.denied'))
            .fontSize(Type.text2)
            .fontColor($r('app.color.fg2'))
            .textAlign(TextAlign.Center)
        }
        .padding({ left: Metrics.page, right: Metrics.page })
      } else if (this.source === Source.Unavailable) {
        // The viewfinder alone, faint, so the card keeps its shape and says where a camera would be
        FinderCorners({ ink: $r('app.color.line_strong') })
        Text(t('mobile.camera.unavailable'))
          .fontSize(Type.text3)
          .fontColor($r('app.color.fg3'))
      }
    }
    .id('pair.camera')
    .width('100%')
    .height(this.cardHeight())
    .backgroundColor($r('app.color.form_row'))
    .clip(true)
  }
}
