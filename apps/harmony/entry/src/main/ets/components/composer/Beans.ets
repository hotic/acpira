import { SessionOption } from '../../wire/Wire';
import {
  accessLabel, ComposerControls, ModelChoices, ModelFamily, modeIcon, modeLabel, optionBrand, Toggle, vendorBrand,
  workLabel
} from '../../logic/ModelCatalog';
import { MentionPersona } from '../../logic/SlashCommands';
import { AttachSource, Goo } from '../../store/Composing';
import { t } from '../../store/L10n';

// What each liquid chip pulls out (apps/ios Design/Goo.swift LiquidAttach / LiquidControls): one bean per setting with
// its value beside it, and the rows its panel holds. Described as data, so the screen's layer (GooLayer) draws every
// bean and panel the same way.

/** A bean's or a row's glyph: a lucide icon, a vendor mark, the sidekick's badged mark, or the Fast bolt */
export interface BeanGlyph {
  /** lucide / mark / sidekick / fast */
  kind: string;
  /** The lucide name or the mark's brand key */
  name: string;
  /** The model's name (a mark without a drawing shows its initial; an empty sidekick label reads as none) */
  label: string;
  /** Ink: fg1, fg3 or warn */
  tint: string;
  on: boolean;
}

/** One choice in a bean's panel: its glyph, its name and the check, or a switch that flips in place */
export interface GooChoice {
  id: string;
  name: string;
  checked: boolean;
  glyph?: BeanGlyph;
  disabled: boolean;
  /** Thinking's switch in place of the check */
  toggle?: Toggle;
  /** Faint text at the row's end (a persona's CLI) */
  note?: string;
  action: () => void;
}

/** One bean: its glyph, the value beside it (and what the setting is under it), and a panel of choices or an action */
export interface Bean {
  key: string;
  title: string;
  caption: string;
  note?: string;
  glyph: BeanGlyph;
  rows: GooChoice[];
  /** The row the panel opens on */
  current: number;
  /** A bean that acts in place (Fast flips, an attach source opens its picker) */
  tap?: () => void;
}

export function lucideGlyph(name: string, tint: string = 'fg1'): BeanGlyph {
  return { kind: 'lucide', name: name, label: '', tint: tint, on: false };
}

export function markGlyph(brand: string | undefined, name: string): BeanGlyph {
  return { kind: 'mark', name: brand ?? '', label: name, tint: 'fg1', on: false };
}

function sidekickGlyph(picked: SessionOption | undefined): BeanGlyph {
  return {
    kind: 'sidekick', name: picked !== undefined ? optionBrand(picked) ?? '' : '', label: picked?.name ?? '', tint: 'fg1',
    on: false
  };
}

function fastGlyph(on: boolean): BeanGlyph {
  return { kind: 'fast', name: 'zap', label: '', tint: 'fg1', on: on };
}

function choice(id: string, name: string, checked: boolean, glyph: BeanGlyph | undefined, action: () => void): GooChoice {
  const row: GooChoice = { id: id, name: name, checked: checked, glyph: glyph, disabled: false, action: action };
  return row;
}

function bean(key: string, title: string, caption: string, glyph: BeanGlyph, rows: GooChoice[], current: number): Bean {
  const out: Bean = { key: key, title: title, caption: caption, glyph: glyph, rows: rows, current: current };
  return out;
}

/** What the model chip's beans act through */
export interface ModelActions {
  setMode: (mode: string) => void;
  setConfig: (id: string, value: string) => void;
}

/**
 * The model chip's beans: the model (or a Fusion pair's lead), the reasoning level, Fast, the working mode, the
 * agent's modes, the approval level, then the sidekick; or the sidekick's own beans while its level is out. A pick folds
 * the panel back into its bean
 */
export function modelBeans(c: ModelChoices, goo: Goo, act: ModelActions): Bean[] {
  const nested = goo.deeper === Goo.sidekickLevel && (c.fusion !== undefined || c.sidekick !== undefined);
  if (nested) {
    return c.fusion !== undefined ? [fusionBean(c, goo, true)] : sidekickBeans(c, goo, act);
  }
  const fold = () => {
    goo.stage = 0;
  };
  const out: Bean[] = [];
  const cur = c.cur;
  if (c.hasModel && cur !== undefined) {
    out.push(bean('model', t('stats.model'), cur.name, markGlyph(cur.brand, cur.name),
      c.families.map((f: ModelFamily) => choice(f.key, f.name, f.key === cur.key, markGlyph(f.brand, f.name), () => {
        c.pickFamily(f);
        fold();
      })), c.families.findIndex((f: ModelFamily) => f.key === cur.key)));
  }
  if (c.fusion !== undefined) {
    out.push(fusionBean(c, goo, false));
  }
  const effort = c.effort;
  if (c.hasEffort && effort !== undefined) {
    const levels = effort.levels.length > 1 ? effort.levels : [];
    const thinking = effort.thinking;
    const name = thinking !== undefined && !thinking.on ? t('mobile.thinking.off') :
      levels.find((l) => l.id === effort.value)?.name ?? (thinking !== undefined ? 'Thinking' : '');
    const rows: GooChoice[] = [];
    if (thinking !== undefined) {
      // The Thinking switch flips in place; the panel stays out
      const row = choice('thinking', 'Thinking', thinking.on, lucideGlyph('brain'), () => {
        if (!thinking.disabled) {
          thinking.set(!thinking.on);
        }
      });
      row.toggle = thinking;
      rows.push(row);
    }
    levels.forEach((l) => rows.push(choice(l.id, l.name, l.id === effort.value, undefined, () => {
      effort.pick(l.id);
      fold();
    })));
    out.push(bean('effort', t('composer.effort'), name,
      lucideGlyph('brain', thinking !== undefined && !thinking.on ? 'fg3' : 'fg1'), rows,
      levels.findIndex((l) => l.id === effort.value)));
  }
  const fast = c.fast;
  if (fast !== undefined) {
    // Every agent's Fast reads the same: the bolt and its name, lit on a tap, no panel
    const b = bean('fast', 'Fast', 'Fast', fastGlyph(fast.on), [], -1);
    b.tap = () => {
      if (!fast.disabled) {
        fast.set(!fast.on);
      }
    };
    out.push(b);
  }
  const work = c.collaboration;
  if (work !== undefined) {
    const current = work.options.find((o: SessionOption) => o.id === work.value) ?? work.options[0];
    out.push(bean('work', work.name, current !== undefined ? workLabel(current) : work.name,
      lucideGlyph(current !== undefined ? modeIcon(current) : 'circleDashed'),
      work.options.map((o: SessionOption) => choice(o.id, workLabel(o), o.id === current?.id, lucideGlyph(modeIcon(o)), () => {
        act.setConfig(work.id, o.id);
        fold();
      })), work.options.findIndex((o: SessionOption) => o.id === current?.id)));
  }
  if (c.modes.length > 0) {
    const mode = c.mode;
    const ink = (m: SessionOption | undefined): string => m?.kind === 'full_access' ? 'warn' : 'fg1';
    out.push(bean('mode', t('composer.mode'), mode !== undefined ? modeLabel(mode) : t('composer.mode'),
      lucideGlyph(mode !== undefined ? modeIcon(mode) : 'circleDashed', ink(mode)),
      c.modes.map((m: SessionOption) => choice(m.id, modeLabel(m), m.id === mode?.id, lucideGlyph(modeIcon(m), ink(m)), () => {
        act.setMode(m.id);
        fold();
      })), c.modes.findIndex((m: SessionOption) => m.id === mode?.id)));
  }
  const access = c.access;
  if (access !== undefined) {
    const level = access.options.find((o: SessionOption) => o.id === access.value) ?? access.options[0];
    out.push(bean('access', t('composer.access'), level !== undefined ? accessLabel(level) : access.name,
      lucideGlyph(level !== undefined ? modeIcon(level) : 'circleDashed'),
      access.options.map((o: SessionOption) => choice(o.id, accessLabel(o), o.id === level?.id, lucideGlyph(modeIcon(o)), () => {
        act.setConfig(access.id, o.id);
        fold();
      })), access.options.findIndex((o: SessionOption) => o.id === level?.id)));
  }
  const enter = () => {
    goo.stage = 0;
    goo.deeper = Goo.sidekickLevel;
  };
  if (c.fusion !== undefined) {
    const name = c.curVar?.sidekick ?? '';
    const picked: SessionOption = { id: name, name: name };
    const b = bean('sidekick', t('composer.sidekick'), name, sidekickGlyph(name.length > 0 ? picked : undefined), [], -1);
    b.note = t('composer.sidekick');
    b.tap = enter;
    out.push(b);
  } else if (c.sidekick !== undefined) {
    const picked = pickedSidekick(c);
    const b = bean('sidekick', t('composer.sidekick'), picked?.name ?? t('composer.sidekickNone'), sidekickGlyph(picked), [],
      -1);
    b.note = t('composer.sidekickHint');
    b.tap = enter;
    out.push(b);
  }
  return out;
}

/** The sidekick picked, none while the select reads None */
function pickedSidekick(c: ModelChoices): SessionOption | undefined {
  const sk = c.sidekick;
  if (sk === undefined) {
    return undefined;
  }
  const current = sk.value ?? ComposerControls.noSidekick;
  return sk.options.find((o: SessionOption) => o.id === current && o.id !== ComposerControls.noSidekick);
}

/**
 * A Fusion pair's lead or sidekick: the built-in sidekick's navigation, every pick writing the real pair's model id.
 * Sidekicks the lead does not pair with stay in the list, disabled
 */
function fusionBean(c: ModelChoices, goo: Goo, sidekick: boolean): Bean {
  const names = (sidekick ? c.fusion?.sidekicks : c.fusion?.leads) ?? [];
  const current = (sidekick ? c.curVar?.sidekick : c.curVar?.lead) ?? names[0] ?? '';
  const picked: SessionOption = { id: current, name: current };
  const title = t(sidekick ? 'composer.sidekick' : 'composer.lead');
  const rows = names.map((name: string) => {
    const row = choice(name, name, name === current, markGlyph(vendorBrand(name), name), () => {
      if (sidekick) {
        c.pickFusion(undefined, name);
      } else {
        c.pickFusion(name, undefined);
      }
      goo.stage = 0;
    });
    row.disabled = sidekick && !c.offersSidekick(name);
    return row;
  });
  const b = bean(sidekick ? 'sidekick.model' : 'lead', title, current,
    sidekick ? sidekickGlyph(current.length > 0 ? picked : undefined) : markGlyph(optionBrand(picked), current), rows,
    names.indexOf(current));
  b.note = title;
  return b;
}

/**
 * The sidekick's own beans, as the lead's read: its model (a list with None first), its reasoning level while the model
 * has levels, and its Fast while the model has a fast tier
 */
function sidekickBeans(c: ModelChoices, goo: Goo, act: ModelActions): Bean[] {
  const sk = c.sidekick;
  if (sk === undefined) {
    return [];
  }
  const fold = () => {
    goo.stage = 0;
  };
  const current = sk.value ?? ComposerControls.noSidekick;
  const picked = pickedSidekick(c);
  const out: Bean[] = [bean('sidekick.model', t('stats.model'), picked?.name ?? t('composer.sidekickNone'),
    sidekickGlyph(picked), sk.options.map((o: SessionOption) => {
      const none = o.id === ComposerControls.noSidekick;
      const glyph = none ? lucideGlyph('circleFadingPlus', 'fg3') : markGlyph(optionBrand(o), o.name);
      return choice(o.id, none ? t('composer.sidekickNone') : o.name, o.id === current, glyph, () => {
        act.setConfig(sk.id, o.id);
        fold();
      });
    }), sk.options.findIndex((o: SessionOption) => o.id === current))];
  const effort = c.sidekickEffort;
  if (picked !== undefined && effort !== undefined && ComposerControls.reasoningVisible(effort)) {
    const scale = ComposerControls.presentReasoning(effort);
    if (scale.efforts.length > 1) {
      const level = scale.off ? t('mobile.thinking.off') :
        scale.efforts.find((o: SessionOption) => o.id === scale.value)?.name ?? t('composer.effort');
      out.push(bean('sidekick.effort', t('composer.effort'), level, lucideGlyph('brain'),
        scale.efforts.map((o: SessionOption) => choice(o.id, o.name, !scale.off && o.id === scale.value, undefined, () => {
          act.setConfig(effort.id, o.id);
          fold();
        })), scale.efforts.findIndex((o: SessionOption) => o.id === scale.value)));
    }
  }
  const fast = c.sidekickFast;
  if (picked !== undefined && fast !== undefined) {
    const b = bean('sidekick.fast', 'Fast', 'Fast', fastGlyph(fast.on), [], -1);
    b.tap = () => {
      if (!fast.disabled) {
        fast.set(!fast.on);
      }
    };
    out.push(b);
  }
  return out;
}

/**
 * `+`'s beans: photos, camera and files squeeze out, each named beside it; the paste bean while the pasteboard holds an
 * image, and the summon list while there are personas to summon
 */
export function attachBeans(goo: Goo, paste: boolean, personas: MentionPersona[],
  onSummon: (persona: MentionPersona) => void): Bean[] {
  const source = (key: string, icon: string, label: string, s: AttachSource): Bean => {
    const b = bean(key, label, label, lucideGlyph(icon), [], -1);
    b.tap = () => {
      goo.close();
      goo.source = s;
    };
    return b;
  };
  const out: Bean[] = [
    source('photos', 'image', t('mobile.attach.photos'), AttachSource.Photos),
    source('camera', 'camera', t('mobile.attach.camera'), AttachSource.Camera),
    source('files', 'paperclip', t('mobile.attach.files'), AttachSource.Files)
  ];
  if (paste) {
    out.push(source('paste', 'clipboardPaste', t('desktop.menu.paste'), AttachSource.Paste));
  }
  if (personas.length > 0) {
    out.push(bean('summon', t('summon.entry'), t('summon.entry'), lucideGlyph('atSign'),
      personas.map((p: MentionPersona) => {
        const row = choice(p.id, p.name, false, p.group ? lucideGlyph('users') : markGlyph(p.agent, p.cli ?? p.name), () => {
          goo.close();
          onSummon(p);
        });
        row.note = p.cli;
        return row;
      }), -1));
  }
  return out;
}
