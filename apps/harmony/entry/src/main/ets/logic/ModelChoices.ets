import { ConfigControl, SessionControls, SessionOption } from '../wire/Wire';
import { t } from '../store/L10n';
import {
  ComposerControls, ControlSplit, Effort, EffortLevel, FusionSides, ModelCatalog, ModelFamily, ModelVariant, Toggle
} from './ModelCatalog';

function nativeToggle(c: ConfigControl, set: (id: string, value: string) => void): Toggle {
  const on = ComposerControls.fastOn(c);
  const next = ComposerControls.fastValue(c, !on);
  return new Toggle(on, !c.options.some((o: SessionOption) => o.id === next), () => set(c.id, next));
}

function level(id: string, name: string): EffortLevel {
  const out: EffortLevel = { id: id, name: name };
  return out;
}

/**
 * What the composer's model chip offers, read from the session's controls (apps/ios Design/ModelChoices.swift): the
 * model families, the reasoning levels with Fast, the modes, the working mode, the approval level, and the sidekick
 * (the built-in agent's own or the one the host gives another CLI's session, found by category; or a Fusion pair's two
 * sides). `hidden` is the agent's settings `hiddenOptions` (control id → family keys)
 */
export class ModelChoices {
  families: ModelFamily[] = [];
  cur: ModelFamily | undefined = undefined;
  curVar: ModelVariant | undefined = undefined;
  modelId: string | undefined = undefined;
  effort: Effort | undefined = undefined;
  mode: SessionOption | undefined = undefined;
  modes: SessionOption[] = [];
  collaboration: ConfigControl | undefined = undefined;
  access: ConfigControl | undefined = undefined;
  agent: ConfigControl | undefined = undefined;
  sidekick: ConfigControl | undefined = undefined;
  sidekickCur: ModelFamily | undefined = undefined;
  sidekickCurVar: ModelVariant | undefined = undefined;
  sidekickEffort: ConfigControl | undefined = undefined;
  sidekickEffortScale: Effort | undefined = undefined;
  sidekickFast: Toggle | undefined = undefined;
  private nativeFast: Toggle | undefined = undefined;
  private onSetConfig: (id: string, value: string) => void;

  constructor(controls: SessionControls | undefined, hidden: Map<string, string[]>,
    onSetConfig: (id: string, value: string) => void) {
    this.onSetConfig = onSetConfig;
    if (controls === undefined) {
      return;
    }
    const split = ComposerControls.split(controls.options);
    // One list of hidden families, the model control's: the sidekick offers the models the model list does, whatever
    // its own control is called (`sidekick` in the built-in agent, `host_sidekick` in another CLI's session)
    const off = split.models.length > 0 ? hidden.get(split.models[0].id) : undefined;
    const model = split.models.length > 0 ? ModelChoices.visible(split.models[0], off) : undefined;
    this.families = model !== undefined ? ModelCatalog.grouped(model.options) : [];
    this.cur = this.families.find((f: ModelFamily) => f.variants.some((v: ModelVariant) => v.id === model?.value));
    this.curVar = this.cur?.variants.find((v: ModelVariant) => v.id === model?.value);
    this.modelId = model?.id;
    const fastControl = split.modelConfig.find((c: ConfigControl) => ComposerControls.isFast(c));
    this.nativeFast = fastControl !== undefined ? nativeToggle(fastControl, onSetConfig) : undefined;
    this.effort = this.effortOf(split);
    this.mode = controls.modes.find((m: SessionOption) => m.id === controls.modeId) ?? controls.modes[0];
    // Beside a permission control the modes have no bean: the working ones are picked from the `/` list (Composer.tsx)
    this.modes = controls.modes.length > 1 && ComposerControls.workingModes(controls) === undefined ? controls.modes : [];
    this.collaboration = split.collaboration.find((c: ConfigControl) => c.options.length > 1);
    this.access = split.permission.find((c: ConfigControl) => c.options.length > 1);
    this.agent = split.agent.find((c: ConfigControl) => c.options.length > 1);
    // Host-owned cross-CLI sidekicks are already filtered per CLI by the engine (`host_sidekick.rs`), so only a native
    // single-agent sidekick takes the lead's hidden list here
    this.sidekick = split.sidekick.length > 0
      ? (ComposerControls.sidekickClis(split.sidekick[0].options).length > 0 ? split.sidekick[0] : ModelChoices.visible(split.sidekick[0], off))
      : undefined;
    const skPicked = this.sidekick !== undefined ? ComposerControls.findSidekickFamily(this.sidekick.options, this.sidekick.value) : undefined;
    this.sidekickCur = skPicked?.family;
    this.sidekickCurVar = skPicked?.variant;
    this.sidekickEffort = split.sidekickEffort[0];
    const nativeSkFast = split.sidekickFast.length > 0 ? nativeToggle(split.sidekickFast[0], onSetConfig) : undefined;
    this.sidekickEffortScale = this.sidekickEffortOf(this.sidekick, this.sidekickCur, this.sidekickCurVar, this.sidekickEffort, nativeSkFast);
    const skFast = this.sidekickEffortScale?.fast ?? nativeSkFast;
    this.sidekickFast = skFast !== undefined && skFast.disabled && !skFast.on ? undefined : skFast;
  }

  /** The models the settings left visible (ModelPicker.tsx visibleOptions) */
  private static visible(c: ConfigControl, hidden: string[] | undefined): ConfigControl {
    const out: ConfigControl = {
      id: c.id, name: c.name, category: c.category, type: c.type, value: c.value,
      options: ModelCatalog.visibleOptions(c.options, hidden, c.value)
    };
    return out;
  }

  get hasModel(): boolean {
    return this.cur !== undefined && this.modelId !== undefined;
  }

  get hasEffort(): boolean {
    return this.effort !== undefined && (this.effort.levels.length > 1 || this.effort.thinking !== undefined);
  }

  /** Fast, when the model in use can switch it at the level in force, or while it is on */
  get fast(): Toggle | undefined {
    const f = this.effort?.fast ?? this.nativeFast;
    return f !== undefined && f.disabled && !f.on ? undefined : f;
  }

  get fusion(): FusionSides | undefined {
    return this.cur?.fusion;
  }

  /** Any bean at all: the chip shows only for an agent that offers something to pick */
  get any(): boolean {
    return this.hasModel || this.hasEffort || this.fast !== undefined || this.collaboration !== undefined ||
      this.modes.length > 0 || this.access !== undefined || this.agent !== undefined || this.sidekick !== undefined;
  }

  /** Keep the lead's effort and the other side where possible; the nearest offered pair wins */
  pickFusion(lead: string | undefined, sidekick: string | undefined): void {
    const f = this.cur;
    const v = this.curVar;
    const id = this.modelId;
    if (f === undefined || f.fusion === undefined || v === undefined || id === undefined) {
      return;
    }
    const hit = ModelCatalog.findFusionVariant(f, lead ?? v.lead ?? '', v.effort, sidekick ?? v.sidekick, v.fast);
    if (hit !== undefined) {
      this.onSetConfig(id, hit.id);
    }
  }

  offersSidekick(name: string): boolean {
    const f = this.cur;
    const v = this.curVar;
    if (f === undefined || v === undefined) {
      return false;
    }
    return f.variants.some((x: ModelVariant) => x.lead === v.lead && x.effort === v.effort && x.sidekick === name);
  }

  /** ModelPanel.pickFamily: into Fusion the model in use becomes the lead; out of it the effort and Fast carry over */
  pickFamily(f: ModelFamily): void {
    const id = this.modelId;
    if (id === undefined) {
      return;
    }
    const v = this.curVar;
    const next = f.fusion !== undefined
      ? ModelCatalog.findFusionVariant(f, v?.lead ?? this.cur?.name ?? '', v?.effort ?? '', v?.sidekick, v?.fast ?? false)
      : v !== undefined ? ModelCatalog.findVariant(f, v.effort, v.fast, false) : undefined;
    const pick = next ?? f.variants[0];
    if (pick !== undefined) {
      this.onSetConfig(id, pick.id);
    }
  }

  private variantFast(f: ModelFamily, v: ModelVariant, controlId: string): Toggle | undefined {
    if (f.fusion !== undefined || !f.hasFast) {
      return undefined;
    }
    const at = (long: boolean): ModelVariant | undefined =>
      f.variants.find((x: ModelVariant) => x.effort === v.effort && x.fast === !v.fast && x.long === long);
    const hit = at(false) ?? at(v.long);
    const set = this.onSetConfig;
    return new Toggle(v.fast, hit === undefined, () => {
      if (hit !== undefined) {
        set(controlId, hit.id);
      }
    });
  }

  private effortOf(split: ControlSplit): Effort | undefined {
    const f = this.cur;
    const v = this.curVar;
    const id = this.modelId;
    const set = this.onSetConfig;
    const nativeFast = this.nativeFast;
    const embeddedFast = f !== undefined && v !== undefined && id !== undefined ? this.variantFast(f, v, id) : undefined;
    let fusionFast: Toggle | undefined = undefined;
    if (f !== undefined && f.fusion !== undefined && v !== undefined && id !== undefined) {
      const pick = (e: string, fast: boolean): void => {
        const hit = ModelCatalog.findFusionVariant(f, v.lead ?? '', e, v.sidekick, fast);
        if (hit !== undefined) {
          set(id, hit.id);
        }
      };
      if (f.hasFast) {
        const can = f.variants.some((x: ModelVariant) => x.lead === v.lead && x.effort === v.effort &&
          x.sidekick === v.sidekick && x.fast !== v.fast);
        fusionFast = new Toggle(v.fast, !can, (on: boolean) => pick(v.effort, on));
      }
      const levels = ModelCatalog.fusionEfforts(f, v);
      if (levels.length > 1) {
        return new Effort(levels.map((e: string) => level(e, e.length === 0 ? t('composer.standard') : e)), v.effort,
          (e: string) => pick(e, v.fast), fusionFast ?? nativeFast, undefined);
      }
    }
    const resolvedFast = fusionFast ?? embeddedFast ?? nativeFast;
    if (f !== undefined && v !== undefined && id !== undefined && f.fusion === undefined &&
      (ModelCatalog.variantScale(f) || ModelCatalog.thinkingOnly(f))) {
      const pick = (e: string): void => {
        const hit = ModelCatalog.findVariant(f, e, v.fast, false);
        if (hit !== undefined) {
          set(id, hit.id);
        }
      };
      // A Standard / Thinking pair is a switch, not two levels
      if (ModelCatalog.thinkingOnly(f)) {
        const on = v.effort === 'Thinking';
        const thinking = new Toggle(on, ModelCatalog.findVariant(f, on ? '' : 'Thinking', v.fast, false) === undefined,
          (next: boolean) => pick(next ? 'Thinking' : ''));
        return new Effort([], v.effort, pick, resolvedFast, thinking);
      }
      return new Effort(f.efforts.map((e: string) => level(e, e.length === 0 ? t('composer.standard') : e)), v.effort, pick,
        resolvedFast, undefined);
    }
    const c = split.reasoning.find((r: ConfigControl) => ComposerControls.reasoningVisible(r));
    if (c !== undefined) {
      const p = ComposerControls.presentReasoning(c);
      // Turned back on at High, else the first level, else the agent's own "on"
      const turnOn = p.efforts.find((o: SessionOption) => o.id === 'high')?.id ?? p.efforts[0]?.id ?? p.onId;
      const offId = p.offId;
      const thinking = offId === undefined ? undefined : new Toggle(!p.off, false, (on: boolean) => {
        if (!on) {
          set(c.id, offId);
        } else if (turnOn !== undefined) {
          set(c.id, turnOn);
        }
      });
      if (p.efforts.length <= 1 && thinking === undefined) {
        const fallbackFast = fusionFast ?? embeddedFast;
        return fallbackFast !== undefined ? new Effort([], v?.effort ?? '', (e: string) => {}, fallbackFast, undefined) : undefined;
      }
      return new Effort(p.efforts.map((o: SessionOption) => level(o.id, o.name)), p.off ? '' : p.value ?? '',
        (e: string) => set(c.id, e), resolvedFast, thinking);
    }
    // A model with embedded Fast (Factory "Opus 5.5 Fast Mode") or pair-level Fast can have Fast without a reasoning bean
    const fallbackFast = fusionFast ?? embeddedFast;
    return fallbackFast !== undefined ? new Effort([], v?.effort ?? '', (e: string) => {}, fallbackFast, undefined) : undefined;
  }

  private sidekickEffortOf(select: ConfigControl | undefined, f: ModelFamily | undefined, v: ModelVariant | undefined,
    nativeEffort: ConfigControl | undefined, nativeFast: Toggle | undefined): Effort | undefined {
    if (select === undefined || f === undefined || v === undefined) {
      return undefined;
    }
    const set = this.onSetConfig;
    const embeddedFast = this.variantFast(f, v, select.id);
    const resolvedFast = embeddedFast ?? nativeFast;
    if (f.fusion === undefined && (ModelCatalog.variantScale(f) || ModelCatalog.thinkingOnly(f))) {
      const pick = (e: string): void => {
        const hit = ModelCatalog.findVariant(f, e, v.fast, false);
        if (hit !== undefined) {
          set(select.id, hit.id);
        }
      };
      if (ModelCatalog.thinkingOnly(f)) {
        const on = v.effort === 'Thinking';
        const thinking = new Toggle(on, ModelCatalog.findVariant(f, on ? '' : 'Thinking', v.fast, false) === undefined,
          (next: boolean) => pick(next ? 'Thinking' : ''));
        return new Effort([], v.effort, pick, resolvedFast, thinking);
      }
      return new Effort(f.efforts.map((e: string) => level(e, e.length === 0 ? t('composer.standard') : e)), v.effort, pick,
        resolvedFast, undefined);
    }
    if (nativeEffort !== undefined && ComposerControls.reasoningVisible(nativeEffort)) {
      const p = ComposerControls.presentReasoning(nativeEffort);
      const turnOn = p.efforts.find((o: SessionOption) => o.id === 'high')?.id ?? p.efforts[0]?.id ?? p.onId;
      const offId = p.offId;
      const thinking = offId === undefined ? undefined : new Toggle(!p.off, false, (on: boolean) => {
        if (!on) {
          set(nativeEffort.id, offId);
        } else if (turnOn !== undefined) {
          set(nativeEffort.id, turnOn);
        }
      });
      if (p.efforts.length > 1 || thinking !== undefined) {
        return new Effort(p.efforts.map((o: SessionOption) => level(o.id, o.name)), p.off ? '' : p.value ?? '',
          (e: string) => set(nativeEffort.id, e), resolvedFast, thinking);
      }
    }
    return embeddedFast !== undefined ? new Effort([], v.effort, (e: string) => {}, embeddedFast, undefined) : undefined;
  }
}
