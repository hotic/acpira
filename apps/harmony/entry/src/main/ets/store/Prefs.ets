import { preferences } from '@kit.ArkData';
import { common } from '@kit.AbilityKit';
import { hiddenSteps, STEP_KINDS } from '../logic/Folding';

// The phone's own preferences (apps/ios Prefs, apps/android's SharedPreferences): the look, the list's grouping, the
// first run, the developer switches and the engine to follow, kept in one @ohos preferences file. The Previewer has no
// preferences store: values then live for the run only.

export class PrefKey {
  /** `system`, `light` or `dark` */
  static readonly theme: string = 'theme';
  /** `auto` or a dictionary id (`en`, `zh-CN` …) */
  static readonly locale: string = 'locale';
  static readonly motion: string = 'motion';
  /** The list's grouping: time | project | agent | none */
  static readonly listView: string = 'listView';
  /** Where pins sit by project: top | project */
  static readonly pinPlace: string = 'pinPlace';
  /** The first-run flow is over, or was never due */
  static readonly welcomed: string = 'welcomed';
  /** The engine to follow: `host:port:token` */
  static readonly engine: string = 'engine';
  /** Notifications were asked for once (the first run or the settings row) */
  static readonly notifyAsked: string = 'notifyAsked';
  /** The process group: the step kinds left out (raw names, comma-separated), step icons, a finished process left open */
  static readonly hiddenSteps: string = 'process.hidden';
  static readonly stepIcons: string = 'process.stepIcons';
  static readonly keepProcess: string = 'process.keepOpen';
  static readonly hudFps: string = 'hud.fps';
  static readonly hudHitches: string = 'hud.hitches';
}

const FILE: string = 'acpira';

export class Prefs {
  private store: preferences.Preferences | undefined = undefined;
  private memory: Map<string, preferences.ValueType> = new Map<string, preferences.ValueType>();

  /** Opens the file; without a store (the Previewer) the values stay in memory */
  open(context: common.Context): void {
    if (this.store !== undefined) {
      return;
    }
    try {
      this.store = preferences.getPreferencesSync(context, { name: FILE });
    } catch (e) {
      this.store = undefined;
    }
  }

  string(key: string, fallback: string): string {
    const v = this.read(key);
    return typeof v === 'string' ? v : fallback;
  }

  bool(key: string, fallback: boolean): boolean {
    const v = this.read(key);
    return typeof v === 'boolean' ? v : fallback;
  }

  set(key: string, value: preferences.ValueType): void {
    this.memory.set(key, value);
    try {
      this.store?.putSync(key, value);
      this.store?.flush();
    } catch (e) {
      // Kept in memory for this run
    }
  }

  remove(key: string): void {
    this.memory.delete(key);
    try {
      this.store?.deleteSync(key);
      this.store?.flush();
    } catch (e) {
      // Nothing stored
    }
  }

  private read(key: string): preferences.ValueType | undefined {
    if (this.memory.has(key)) {
      return this.memory.get(key);
    }
    try {
      const v = this.store?.getSync(key, '\u0000');
      return v === '\u0000' ? undefined : v;
    } catch (e) {
      return undefined;
    }
  }
}

/** The preferences the whole app shares */
export const prefs: Prefs = new Prefs();

/**
 * The settings' process group (Appearance.tsx, apps/ios ProcessPrefs): the step kinds left out, whether steps keep
 * their lead icons, whether a finished turn's process stays open. The transcript reads it wherever it draws a process
 */
@ObservedV2
export class ProcessLook {
  @Trace hidden: string[] = [];
  @Trace stepIcons: boolean = true;
  @Trace keepProcess: boolean = false;

  /** Read back once the preferences file is open */
  load(): void {
    this.hidden = hiddenSteps(prefs.string(PrefKey.hiddenSteps, ''));
    this.stepIcons = prefs.bool(PrefKey.stepIcons, true);
    this.keepProcess = prefs.bool(PrefKey.keepProcess, false);
  }

  /** One step kind shown or left out */
  show(kind: string, on: boolean): void {
    this.hidden = STEP_KINDS.filter((k: string) => k === kind ? !on : this.hidden.includes(k));
    prefs.set(PrefKey.hiddenSteps, this.hidden.join(','));
  }

  setStepIcons(on: boolean): void {
    this.stepIcons = on;
    prefs.set(PrefKey.stepIcons, on);
  }

  setKeepProcess(on: boolean): void {
    this.keepProcess = on;
    prefs.set(PrefKey.keepProcess, on);
  }
}

export const processLook: ProcessLook = new ProcessLook();
