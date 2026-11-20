import { SessionOption } from '../../wire/Wire';
import {
  accessIcon, accessLabel, ComposerControls, EffortLevel, FamilyChannelGroup, ModelChoices, SidekickCli, ModelFamily, ModelVariant, modeIcon, modeLabel, optionBrand, Toggle, vendorBrand,
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
  /** A label over this row, the channel it and the rows after it come through (a sidekick model list's group label) */
  section?: string;
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
    out.push(bean('mode', t('composer.mode'), mode !== undefined ? modeLabel(mode) : t('composer.mode'),
      lucideGlyph(mode !== undefined ? modeIcon(mode) : 'circleDashed'),
      c.modes.map((m: SessionOption) => choice(m.id, modeLabel(m), m.id === mode?.id, lucideGlyph(modeIcon(m)), () => {
        act.setMode(m.id);
        fold();
      })), c.modes.findIndex((m: SessionOption) => m.id === mode?.id)));
  }
  const access = c.access;
  if (access !== undefined) {
    const level = access.options.find((o: SessionOption) => o.id === access.value) ?? access.options[0];
    out.push(bean('access', t('composer.access'), level !== undefined ? accessLabel(level) : access.name,
      lucideGlyph(level !== undefined ? accessIcon(level) : 'circleDashed'),
      access.options.map((o: SessionOption) => choice(o.id, accessLabel(o), o.id === level?.id, lucideGlyph(accessIcon(o)), () => {
        act.setConfig(access.id, o.id);
        fold();
      })), access.options.findIndex((o: SessionOption) => o.id === level?.id)));
  }
  const agent = c.agent;
  if (agent !== undefined) {
    const current = agent.options.find((o: SessionOption) => o.id === agent.value) ?? agent.options[0];
    out.push(bean('agent', agent.name, current !== undefined ? current.name : agent.name,
      lucideGlyph(current !== undefined ? modeIcon(current) : 'circleDashed'),
      agent.options.map((o: SessionOption) => choice(o.id, o.name, o.id === current?.id, lucideGlyph(modeIcon(o)), () => {
        act.setConfig(agent.id, o.id);
        fold();
      })), agent.options.findIndex((o: SessionOption) => o.id === current?.id)));
  }
  // None yet, with models from two or more CLIs: the sidekick bean goes straight to the CLI list, not to a lone agent bean
  const toAgents = c.fusion === undefined && c.sidekick !== undefined && pickedSidekick(c) === undefined &&
    ComposerControls.sidekickClis(c.sidekick.options).length > 1;
  const enter = () => {
    goo.browse = undefined;
    goo.stage = toAgents ? 1 : 0;
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
    const b = bean('sidekick', t('composer.sidekick'), c.sidekickCur?.name ?? picked?.name ?? t('composer.sidekickNone'), sidekickGlyph(picked), [],
      -1);
    b.note = t('composer.sidekickHint');
    b.tap = enter;
    out.push(b);
  }
  return out;
}

/**
 * The CLI the sidekick runs in: its mark and name (a faint fading plus and None while there is none), and a panel of
 * None and every CLI. None turns the sidekick off; a CLI is no pick of its own, so it opens the model bean (the second)
 * on that CLI's models
 */
function agentBean(clis: SidekickCli[], picked: SidekickCli | undefined, goo: Goo, none: () => void): Bean {
  const rows: GooChoice[] = [choice(ComposerControls.noSidekick, t('composer.sidekickNone'), picked === undefined,
    lucideGlyph('circleFadingPlus', 'fg3'), none)];
  clis.forEach((cli: SidekickCli) => {
    rows.push(choice(cli.id, cli.name, cli.id === picked?.id, markGlyph(cli.id, cli.name), () => {
      goo.browse = cli.id;
      goo.stage = 2;
    }));
  });
  const glyph = picked !== undefined ? markGlyph(picked.id, picked.name) : lucideGlyph('circleFadingPlus', 'fg3');
  return bean('sidekick.agent', t('common.agent'), picked?.name ?? t('composer.sidekickNone'), glyph, rows,
    picked !== undefined ? clis.indexOf(picked) + 1 : 0);
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
 * The sidekick's own beans, as the lead's read: with models from two or more CLIs (the host's select) first the CLI, as
 * the desktop's rail is, then its model (one CLI's models, filed by channel, so a group label always names a channel),
 * its reasoning level while the model has levels, and its Fast while the model has a fast tier. With one CLI the model
 * list holds None first. Every pick writes the id of the control found by category
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
  const pickedFamily = c.sidekickCur;
  const clis = ComposerControls.sidekickClis(sk.options);
  const out: Bean[] = [];
  let activeCli: SidekickCli | undefined = clis.length === 1 ? clis[0] : undefined;
  if (clis.length > 1) {
    const mine = clis.find((cli: SidekickCli) => cli.id === picked?.group?.id);
    out.push(agentBean(clis, mine, goo, () => {
      act.setConfig(sk.id, ComposerControls.noSidekick);
      goo.browse = undefined;
      fold();
    }));
    activeCli = clis.find((cli: SidekickCli) => cli.id === goo.browse) ?? mine;
  }
  const listed: SessionOption[] | undefined = clis.length > 1 ? activeCli?.options : sk.options;
  const cliId = activeCli?.id;
  if (listed !== undefined) {
    const showNone = clis.length <= 1;
    const families = ComposerControls.sidekickFamilies(listed, cliId);
    const groups = ComposerControls.byChannelFamilies(families);
    const ordered = groups !== undefined
      ? groups.reduce((all: ModelFamily[], g: FamilyChannelGroup) => all.concat(g.families), [] as ModelFamily[])
      : families;
    const rows: GooChoice[] = [];
    if (showNone) {
      rows.push(choice(ComposerControls.noSidekick, t('composer.sidekickNone'), picked === undefined,
        lucideGlyph('circleFadingPlus', 'fg3'), () => {
          act.setConfig(sk.id, ComposerControls.noSidekick);
          fold();
        }));
    }
    ordered.forEach((f: ModelFamily) => {
      const checked = f.variants.some((v: ModelVariant) => v.id === current);
      const target = f.variants[0]?.id ?? f.key;
      const row = choice(f.key, f.name, checked, markGlyph(f.brand, f.name), () => {
        act.setConfig(sk.id, target);
        fold();
      });
      const g = groups?.find((group: FamilyChannelGroup) => group.families[0]?.key === f.key);
      row.section = g !== undefined && g.channel.length > 0 ? g.channel : undefined;
      rows.push(row);
    });
    const activeIndex = picked === undefined ? 0
      : ordered.findIndex((f: ModelFamily) => f.variants.some((v: ModelVariant) => v.id === current)) + (showNone ? 1 : 0);
    out.push(bean('sidekick.model', t('stats.model'), pickedFamily?.name ?? picked?.name ?? t('composer.sidekickNone'),
      sidekickGlyph(picked), rows, activeIndex));
  }
  const e = picked !== undefined ? c.sidekickEffortScale : undefined;
  if (e !== undefined && (e.levels.length > 1 || e.thinking !== undefined)) {
    const levels = e.levels.length > 1 ? e.levels : [];
    const thinking = e.thinking;
    const label = thinking !== undefined && !thinking.on ? t('mobile.thinking.off')
      : levels.find((l: EffortLevel) => l.id === e.value)?.name ?? (thinking !== undefined ? 'Thinking' : t('composer.effort'));
    const rows: GooChoice[] = [];
    if (thinking !== undefined) {
      rows.push(choice('off', t('mobile.thinking.off'), !thinking.on, undefined, () => {
        thinking.set(false);
        fold();
      }));
      if (levels.length === 0) {
        rows.push(choice('on', 'Thinking', thinking.on, undefined, () => {
          thinking.set(true);
          fold();
        }));
      }
    }
    levels.forEach((l: EffortLevel) => {
      rows.push(choice(l.id, l.name, (thinking === undefined || thinking.on) && l.id === e.value, undefined, () => {
        e.pick(l.id);
        fold();
      }));
    });
    const active = thinking !== undefined && !thinking.on ? 0
      : levels.findIndex((l: EffortLevel) => l.id === e.value) + (thinking !== undefined ? 1 : 0);
    out.push(bean('sidekick.effort', t('composer.effort'), label, lucideGlyph('brain'), rows, active));
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
