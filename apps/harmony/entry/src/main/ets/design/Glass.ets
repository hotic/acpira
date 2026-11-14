import { uiMaterial } from '@kit.ArkUI';
import { deviceInfo } from '@kit.BasicServicesKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { ListMetrics, Metrics } from './Tokens';

// The system's glass (the immersive material, API 26) and its stand-in before it, as apps/ios
// AcpiraDesign/Compat26.swift does for Liquid Glass: one build serves both, and each surface asks the running system, so
// a device updated to 26 gets the material without a new release. Views take a modifier from here, never
// `systemMaterial` itself, which a system before 26 does not have.

const DOMAIN = 0xacb1;
const TAG = 'AcpiraGlass';
/** The API level the material came with */
const MATERIAL_API = 26;
let supported: boolean | undefined = undefined;

/** Whether the running system draws the glass itself; asked once */
export function systemGlass(): boolean {
  if (supported === undefined) {
    try {
      supported = deviceInfo.sdkApiVersion >= MATERIAL_API && uiMaterial.isImmersiveMaterialSupported();
      hilog.info(DOMAIN, TAG, 'system material: %{public}s (API %{public}d)', `${supported}`, deviceInfo.sdkApiVersion);
    } catch (e) {
      supported = false;
    }
  }
  return supported;
}

/** What a glass surface's stand-in is drawn with besides its fill */
export enum GlassEdge {
  None,
  Line,
  Lifted
}

/**
 * A glass surface: the system's material where it has one (which brings its own fill, edge and shadow, so none is set
 * beside it), else the flat fill with this edge. The caller keeps the shape (size, corner radius)
 */
export class Glass implements AttributeModifier<CommonAttribute> {
  private edge: GlassEdge;
  /** Whether the surface is one control: the material then gives under a finger */
  private pressable: boolean;
  /** Built on first use, and only where the class exists */
  private material: uiMaterial.Material | undefined = undefined;

  constructor(edge: GlassEdge, pressable: boolean) {
    this.edge = edge;
    this.pressable = pressable;
  }

  applyNormalAttribute(instance: CommonAttribute): void {
    if (systemGlass()) {
      // A system that says it has the material and then refuses it keeps the stand-in
      try {
        if (this.material === undefined) {
          this.material = new uiMaterial.ImmersiveMaterial({
            interactive: this.pressable,
            applyShadow: this.edge === GlassEdge.Lifted
          });
        }
        instance.systemMaterial(this.material);
        return;
      } catch (e) {
        hilog.warn(DOMAIN, TAG, 'no system material: %{public}s', `${e}`);
      }
    }
    instance.backgroundColor($r('app.color.glass_bg'));
    if (this.edge !== GlassEdge.None) {
      instance.border({ width: Metrics.hairline, color: $r('app.color.glass_line') });
    }
    if (this.edge === GlassEdge.Lifted) {
      instance.shadow({ radius: ListMetrics.barShadow, color: $r('app.color.line'), offsetY: Metrics.s1 / 2 });
    }
  }
}

/** A bar's round control, and a capsule or field that holds several */
export const GLASS_CONTROL: Glass = new Glass(GlassEdge.Lifted, true);
export const GLASS_BAR: Glass = new Glass(GlassEdge.Lifted, false);
/** A pill under a hairline, and a bare one (the new session's tiles) */
export const GLASS_PILL: Glass = new Glass(GlassEdge.Line, true);
export const GLASS_BARE: Glass = new Glass(GlassEdge.None, true);
