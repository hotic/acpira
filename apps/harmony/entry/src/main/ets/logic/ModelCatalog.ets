import { ConfigControl, SessionControls, SessionOption } from '../wire/Wire';
import { t } from '../store/L10n';

// apps/ios Logic/ModelCatalog.swift, Design/Goo.swift ModelChoices and Chat/Composer.swift modeIcon / modeLabel
// (shared/models.ts, shared/composerControls.ts, modeIcons.ts): the model list rebuilt from flat option names (Devin
// flattens model × effort × Fast × 1M into one select, and Fusion pairs into "Fusion (lead + sidekick)"), the native
// reasoning / Fast controls, the sidekick, the built-in agent's approval level, and what the model chip offers. The
// wire ids stay untouched; only the presentation is derived.

/** One option of a family: the wire id plus the dimensions parsed from its name */
export interface ModelVariant {
  id: string;
  name: string;
  /** None / Minimal / Low / Medium / High / XHigh / Max / Thinking; "" is the level-less (Standard) variant */
  effort: string;
  fast: boolean;
  long: boolean;
  /** Fusion only: the lead family (effort / fast describe it) and the sidekick label ("SWE-2 High") */
  lead?: string;
  sidekick?: string;
}

export interface FusionSides {
  leads: string[];
  sidekicks: string[];
}

/** A model family: one row of the model panel, its variants the params underneath */
export interface ModelFamily {
  /** Namespaced identity (the TS key verbatim: settings `hiddenOptions` stores it) */
  key: string;
  name: string;
  brand?: string;
  source?: string;
  sourceKind?: string;
  variants: ModelVariant[];
  /** Effort labels by strength, deduplicated */
  efforts: string[];
  hasFast: boolean;
  hasLong: boolean;
  fusion?: FusionSides;
}

interface Parsed {
  family: string;
  effort: string;
  fast: boolean;
  long: boolean;
  lead?: string;
  sidekick?: string;
}

interface EffortTokens {
  rest: string[];
  effort: string;
}

interface BrandRule {
  pattern: RegExp;
  brand: string;
}

/** Model family → brand key (Marks.generated.swift brands), first match wins on the lowercased name */
const BRANDS: BrandRule[] = [
  { pattern: /\bclaude(?![a-z])|\bopus\b|\bsonnet\b|\bhaiku\b|\bfable\b/, brand: 'claude' },
  { pattern: /\bglm(?![a-z])|\bzhipu\b|\bchatglm(?![a-z])|\bzai\b|\bz\.ai\b/, brand: 'zai' },
  { pattern: /\bkimi(?![a-z])|\bmoonshot\b/, brand: 'kimi' },
  { pattern: /\bk\d+(\.\d+)?\b/, brand: 'kimi' },
  { pattern: /\bswe\b|\bwindsurf\b/, brand: 'windsurf' },
  { pattern: /\badaptive\b|\bdevin\b|\bfusion\b/, brand: 'devin' },
  { pattern: /\bcogito(?![a-z])/, brand: 'cogito' },
  { pattern: /\bnemotron(?![a-z])|\bnvidia\b/, brand: 'nvidia' },
  { pattern: /\bsonar(?![a-z])|\bperplexity\b|\br1-1776\b/, brand: 'perplexity' },
  { pattern: /\bcomposer(?![a-z])/, brand: 'cursor' },
  { pattern: /\bbeam(?![a-z])|\breflection\b/, brand: 'reflection' },
  { pattern: /\binkling(?![a-z])|\bthinking[- ]?machines\b/, brand: 'thinkingmachines' },
  { pattern: /\bflint(?![a-z])|\brecursive(?:si)?\b/, brand: 'recursive' },
  {
    pattern: /\bgpt(?![a-z])|\bopenai\b|\bcodex\b|\bchatgpt\b|\bo\d(?![a-z])|\bdall-?e(?![a-z])|\bwhisper(?![a-z])|\btts-\d|\btext-embedding\b|\bsora(?![a-z])/,
    brand: 'openai'
  },
  { pattern: /\bgemini(?![a-z])|\bgemma(?![a-z])/, brand: 'gemini' },
  { pattern: /\bveo(?![a-z])|\blyria(?![a-z])|\bimagen(?![a-z])|\bdeep-research\b|\bnano-?banana\b|\bpalm(?![a-z])/, brand: 'google' },
  { pattern: /\bgrok(?![a-z])/, brand: 'grok' },
  { pattern: /\bdeepseek(?![a-z])/, brand: 'deepseek' },
  { pattern: /\bqwen(?![a-z])|\btongyi\b|\bqwq(?![a-z])|\bqvq(?![a-z])/, brand: 'qwen' },
  { pattern: /\b(?:mistral|mixtral|codestral|devstral|ministral|magistral|pixtral|voxtral|leanstral)(?![a-z])/, brand: 'mistral' },
  { pattern: /\bminimax(?![a-z])|\babab\d/, brand: 'minimax' },
  { pattern: /\bmuse(?![a-z])|\bllama(?![a-z])/, brand: 'meta' },
  { pattern: /\bcommand(?![a-z])|\bcohere\b|\baya(?![a-z])/, brand: 'cohere' },
  { pattern: /\bphi(?![a-z])|\bmai-ds(?![a-z])/, brand: 'microsoft' },
  { pattern: /\bnova(?![a-z])|\bamazon\b/, brand: 'aws' },
  { pattern: /\bdoubao(?![a-z])/, brand: 'doubao' },
  { pattern: /\bseed(?![a-z])|\bui-tars\b|\bbytedance\b/, brand: 'bytedance' },
  { pattern: /\bhunyuan(?![a-z])/, brand: 'hunyuan' },
  { pattern: /\bmimo(?![a-z])/, brand: 'mimo' }
];

/** The brand key of a model name ("Claude Fable 5.1" → claude), undefined for an unknown vendor */
export function vendorBrand(name: string): string | undefined {
  const lower = name.toLowerCase();
  for (const rule of BRANDS) {
    if (rule.pattern.test(lower)) {
      return rule.brand;
    }
  }
  return undefined;
}

/** The vendor of an option: its wire id is the real identity ("asgard/kimi-k3"), the name only a label */
export function optionBrand(o: SessionOption): string | undefined {
  return vendorBrand(o.id) ?? vendorBrand(o.name);
}

const LEVELS: Map<string, string> = new Map<string, string>([
  ['none', 'None'], ['minimal', 'Minimal'], ['low', 'Low'], ['medium', 'Medium'], ['high', 'High'], ['xhigh', 'XHigh'],
  ['x-high', 'XHigh'], ['max', 'Max']
]);
const EFFORT_ORDER: string[] = ['', 'Thinking', 'None', 'Minimal', 'Low', 'Medium', 'High', 'XHigh', 'Max'];
const FUSION = /^Fusion\s*\((.+?)\s+\+\s+(.+)\)$/i;

export class ModelCatalog {
  static rank(effort: string): number {
    const at = EFFORT_ORDER.indexOf(effort);
    return at >= 0 ? at : EFFORT_ORDER.length;
  }

  /**
   * "Fusion (GPT-6 Astra High Thinking Fast + GPT-5.6 Luna High Thinking Fast)": lead GPT-6 Astra at High, sidekick
   * "GPT-5.6 Luna High", Fast from either side (one pair-level switch on the wire)
   */
  static parseFusion(name: string): Parsed | undefined {
    const m = name.trim().match(FUSION);
    if (m === null) {
      return undefined;
    }
    const lead = ModelCatalog.parse(m[1]);
    const sk = ModelCatalog.parse(m[2]);
    const parsed: Parsed = {
      family: 'Fusion', effort: lead.effort, fast: lead.fast || sk.fast, long: lead.long, lead: lead.family,
      sidekick: [sk.family, sk.effort].filter((s: string) => s.length > 0).join(' ')
    };
    return parsed;
  }

  private static effortTokens(tokens: string[]): EffortTokens {
    const w = tokens.slice();
    let thinking = false;
    let effort = '';
    if (w.length > 0 && w[w.length - 1].toLowerCase() === 'thinking') {
      thinking = true;
      w.pop();
    }
    const last = w.length > 0 ? w[w.length - 1].toLowerCase() : '';
    if (last === 'high' && w.length >= 2 && w[w.length - 2].toLowerCase() === 'extra') {
      effort = 'XHigh';
      w.pop();
      w.pop();
    } else if (LEVELS.has(last)) {
      effort = LEVELS.get(last) ?? '';
      w.pop();
    } else if (last === 'no' && thinking) {
      effort = 'None';
      thinking = false;
      w.pop();
    }
    if (effort.length === 0 && thinking) {
      effort = 'Thinking';
    }
    const out: EffortTokens = { rest: w, effort: effort };
    return out;
  }

  /** `<family> [effort] [Thinking] [Fast] [1M]`, the effort also in parentheses */
  static parse(name: string): Parsed {
    const fusion = ModelCatalog.parseFusion(name);
    if (fusion !== undefined) {
      return fusion;
    }
    let tk: string[] = name.trim().split(/\s+/).filter((s: string) => s.length > 0);
    let fast = false;
    let long = false;
    let effort = '';
    // Fast and 1M may both trail, in either order
    const stripFlags = () => {
      while (tk.length > 1) {
        const last = tk[tk.length - 1].toLowerCase();
        // Keep compound "Ultra Fast" tiers ("GPT-6.2 Sol Ultra Fast") intact as their own family
        if (last === 'fast' && tk[tk.length - 2].toLowerCase() !== 'ultra') {
          fast = true;
          tk.pop();
        } else if (tk.length > 2 && last === 'mode' && tk[tk.length - 2].toLowerCase() === 'fast') {
          // Droid / Factory spells it "Opus 5.5 Fast Mode"
          fast = true;
          tk.pop();
          tk.pop();
        } else if (last === '1m') {
          long = true;
          tk.pop();
        } else {
          break;
        }
      }
    };
    stripFlags();
    if (tk.length > 1 && tk[tk.length - 1].endsWith(')')) {
      let open = -1;
      for (let i = tk.length - 1; i >= 1; i--) {
        if (tk[i].startsWith('(')) {
          open = i;
          break;
        }
      }
      if (open >= 0) {
        const group = tk.slice(open);
        group[0] = group[0].slice(1);
        group[group.length - 1] = group[group.length - 1].slice(0, -1);
        const inner = group.filter((s: string) => s.length > 0);
        if (inner.length > 0 && inner.every((s: string) => !s.includes('(') && !s.includes(')'))) {
          const p = ModelCatalog.effortTokens(inner);
          if (p.effort.length > 0 && p.rest.length === 0) {
            effort = p.effort;
            tk = tk.slice(0, open);
            stripFlags();
          }
        }
      }
    }
    if (effort.length === 0 && tk.length > 1) {
      const p = ModelCatalog.effortTokens(tk.slice(1));
      if (p.effort.length > 0) {
        effort = p.effort;
        tk = [tk[0]].concat(p.rest);
        stripFlags();
      }
    }
    const out: Parsed = { family: tk.join(' '), effort: effort, fast: fast, long: long };
    return out;
  }

  /** `JSON.stringify` of a short array of strings and nulls, as models.ts builds a family's key */
  static jsonKey(parts: (string | null)[]): string {
    return JSON.stringify(parts);
  }

  private static lastOptions: SessionOption[] | undefined = undefined;
  private static lastFamilies: ModelFamily[] = [];

  /** `group` for the chip, which reads the same list on every redraw: the last list's families are kept */
  static grouped(options: SessionOption[]): ModelFamily[] {
    if (ModelCatalog.lastOptions === options) {
      return ModelCatalog.lastFamilies;
    }
    const families = ModelCatalog.group(options);
    ModelCatalog.lastOptions = options;
    ModelCatalog.lastFamilies = families;
    return families;
  }

  /** Options grouped into families in first-seen order, variants sorted by lead, effort, sidekick, Fast, 1M */
  static group(options: SessionOption[]): ModelFamily[] {
    // With no provider metadata, equal parameter tuples are ambiguous, not interchangeable
    const tuples: Set<string> = new Set<string>();
    const ambiguous: Set<string> = new Set<string>();
    const parsed: Parsed[] = options.map((o: SessionOption) => ModelCatalog.parse(o.name));
    options.forEach((o: SessionOption, i: number) => {
      const p = parsed[i];
      const base = JSON.stringify([o.source?.id ?? o.group?.id ?? null, p.family]);
      const tuple = JSON.stringify([base, p.effort, p.fast, p.long, p.lead ?? null, p.sidekick ?? null]);
      if (tuples.has(tuple)) {
        ambiguous.add(base);
      }
      tuples.add(tuple);
    });
    const order: string[] = [];
    const map: Map<string, ModelFamily> = new Map<string, ModelFamily>();
    options.forEach((o: SessionOption, i: number) => {
      const p = parsed[i];
      const namespace: string | undefined = o.source?.id ?? o.group?.id;
      const separate = ambiguous.has(JSON.stringify([namespace ?? null, p.family]));
      const source: string | undefined = o.source?.name ?? o.group?.name ?? (separate ? o.id : undefined);
      const key = separate ? ModelCatalog.jsonKey([namespace ?? null, p.family, o.id])
        : namespace !== undefined ? ModelCatalog.jsonKey([namespace, p.family]) : p.family;
      let f = map.get(key);
      if (f === undefined) {
        order.push(key);
        const made: ModelFamily = {
          key: key, name: p.family, source: source, sourceKind: o.source?.kind, variants: [], efforts: [], hasFast: false,
          hasLong: false
        };
        f = made;
        map.set(key, made);
      }
      // Fusion pairs are Devin's own routing, like Adaptive
      if (f.brand === undefined) {
        f.brand = p.lead !== undefined ? 'devin' : optionBrand(o);
      }
      if (p.lead !== undefined) {
        const fu: FusionSides = f.fusion ?? { leads: [], sidekicks: [] };
        if (!fu.leads.includes(p.lead)) {
          fu.leads.push(p.lead);
        }
        if (p.sidekick !== undefined && !fu.sidekicks.includes(p.sidekick)) {
          fu.sidekicks.push(p.sidekick);
        }
        f.fusion = fu;
      }
      const v: ModelVariant = {
        id: o.id, name: o.name, effort: p.effort, fast: p.fast, long: p.long, lead: p.lead,
        sidekick: p.lead !== undefined ? p.sidekick : undefined
      };
      f.variants.push(v);
    });
    return order.map((key: string) => {
      const f = map.get(key) as ModelFamily;
      const fusion = f.fusion;
      const leadRank = (v: ModelVariant): number => Math.max(0, fusion?.leads.indexOf(v.lead ?? '') ?? 0);
      const skRank = (v: ModelVariant): number => Math.max(0, fusion?.sidekicks.indexOf(v.sidekick ?? '') ?? 0);
      f.variants.sort((a: ModelVariant, b: ModelVariant): number => {
        const keys: number[] = [leadRank(a) - leadRank(b), ModelCatalog.rank(a.effort) - ModelCatalog.rank(b.effort),
          skRank(a) - skRank(b), (a.fast ? 1 : 0) - (b.fast ? 1 : 0), (a.long ? 1 : 0) - (b.long ? 1 : 0)];
        return keys.find((k: number) => k !== 0) ?? 0;
      });
      const seen: Set<string> = new Set<string>();
      f.efforts = f.variants.map((v: ModelVariant) => v.effort).filter((e: string) => {
        if (seen.has(e)) {
          return false;
        }
        seen.add(e);
        return true;
      }).sort((a: string, b: string) => ModelCatalog.rank(a) - ModelCatalog.rank(b));
      f.hasFast = f.variants.some((v: ModelVariant) => v.fast);
      f.hasLong = f.variants.some((v: ModelVariant) => v.long);
      return f;
    });
  }

  /** Exact effort / Fast / 1M first, then dropping 1M, Fast, both; finally any variant at that effort */
  static findVariant(f: ModelFamily, effort: string, fast: boolean, long: boolean): ModelVariant | undefined {
    const hit = (fa: boolean, lo: boolean): ModelVariant | undefined =>
      f.variants.find((v: ModelVariant) => v.effort === effort && v.fast === fa && v.long === lo);
    return hit(fast, long) ?? hit(fast, false) ?? hit(false, long) ?? hit(false, false) ??
      f.variants.find((v: ModelVariant) => v.effort === effort);
  }

  /** The offered Fusion pair nearest to lead × effort × sidekick × fast */
  static findFusionVariant(f: ModelFamily, lead: string, effort: string, sidekick: string | undefined,
    fast: boolean): ModelVariant | undefined {
    const ofLead = f.variants.filter((v: ModelVariant) => v.lead === lead);
    const pool = ofLead.length === 0 ? f.variants : ofLead;
    const hit = (e: string, sk: string | undefined, fa: boolean): ModelVariant | undefined =>
      pool.find((v: ModelVariant) => v.effort === e && (sk === undefined || v.sidekick === sk) && v.fast === fa);
    const at = (e: string): ModelVariant | undefined =>
      hit(e, sidekick, fast) ?? hit(e, sidekick, false) ?? hit(e, undefined, fast) ?? hit(e, undefined, false);
    const seen: Set<string> = new Set<string>();
    const nearest = pool.map((v: ModelVariant) => v.effort).filter((e: string) => {
      if (e === effort || seen.has(e)) {
        return false;
      }
      seen.add(e);
      return true;
    }).sort((a: string, b: string): number => {
      const da = Math.abs(ModelCatalog.rank(a) - ModelCatalog.rank(effort));
      const db = Math.abs(ModelCatalog.rank(b) - ModelCatalog.rank(effort));
      return da !== db ? da - db : ModelCatalog.rank(b) - ModelCatalog.rank(a);
    });
    const direct = at(effort);
    if (direct !== undefined) {
      return direct;
    }
    for (const e of nearest) {
      const v = at(e);
      if (v !== undefined) {
        return v;
      }
    }
    return pool[0];
  }

  /** A Standard / Thinking pair is a Thinking switch rather than two levels */
  static thinkingOnly(f: ModelFamily): boolean {
    return f.efforts.length === 2 && f.efforts.includes('') && f.efforts.includes('Thinking');
  }

  static variantScale(f: ModelFamily): boolean {
    return f.efforts.length > 1 && !ModelCatalog.thinkingOnly(f);
  }

  static fusionEfforts(f: ModelFamily, v: ModelVariant): string[] {
    const lead = v.lead ?? f.fusion?.leads[0] ?? '';
    return f.efforts.filter((e: string) => f.variants.some((x: ModelVariant) => x.lead === lead && x.effort === e));
  }

  /** ModelVisibility.visibleOptions: hidden families dropped, the current value always kept, never an empty list */
  static visibleOptions(options: SessionOption[], hidden: string[] | undefined, current: string | undefined): SessionOption[] {
    if (hidden === undefined || hidden.length === 0) {
      return options;
    }
    const off: Set<string> = new Set<string>();
    for (const f of ModelCatalog.group(options)) {
      const legacy: string[] = f.source !== undefined ? [f.name, `${f.source}/${f.name}`] : [f.name];
      const allNamed = f.variants.length > 1 && f.variants.every((v: ModelVariant) => hidden.includes(v.name));
      if (hidden.includes(f.key) || legacy.some((k: string) => hidden.includes(k)) || allNamed) {
        f.variants.forEach((v: ModelVariant) => off.add(v.id));
      }
    }
    const kept = options.filter((o: SessionOption) => o.id === current || !off.has(o.id));
    return kept.length === 0 ? options : kept;
  }
}

/** Presentation of an agent's controls whatever the wire shape (composerControls.ts) */
const EFFORT_LABELS: string[] = ['None', 'Minimal', 'Low', 'Medium', 'High', 'XHigh', 'Max', 'Ultra', 'Thinking'];

export interface Reasoning {
  efforts: SessionOption[];
  offId?: string;
  onId?: string;
  value?: string;
  off: boolean;
}

/** The toolbar's split of an agent's controls */
export interface ControlSplit {
  models: ConfigControl[];
  reasoning: ConfigControl[];
  modelConfig: ConfigControl[];
  collaboration: ConfigControl[];
  permission: ConfigControl[];
  agent: ConfigControl[];
  sidekick: ConfigControl[];
  sidekickEffort: ConfigControl[];
  sidekickFast: ConfigControl[];
}

/** One CLI a sidekick can run in, with its models in the select's order */
export interface SidekickCli {
  id: string;
  name: string;
  options: SessionOption[];
}

/** A run of sidekick model families under one channel ('' for those without one) */
export interface FamilyChannelGroup {
  channel: string;
  families: ModelFamily[];
}

/** A resolved sidekick pick: its model family, active variant and CLI when cross-CLI */
export interface PickedSidekick {
  family: ModelFamily;
  variant: ModelVariant;
  cli?: SidekickCli;
}

/** A run of sidekick models under one channel ('' for those without one) */
export interface ChannelGroup {
  channel: string;
  options: SessionOption[];
}

export class ComposerControls {
  static readonly sidekickCategory: string = '_sidekick';
  static readonly sidekickEffortCategory: string = '_sidekick_effort';
  static readonly sidekickFastCategory: string = '_sidekick_fast';
  /** The sidekick's value for none */
  static readonly noSidekick: string = 'none';
  /** A permission level offered as a config option (the built-in agent's approval, Claude's levels split off its modes) */
  static readonly permissionCategory: string = '_permission_mode';

  /**
   * Composer.tsx `split`: beside a permission control the modes get no bean. The first is the resting one; the rest are
   * working modes, entered by name from the `/` list and shown as a pill while in force. undefined without that split
   */
  static workingModes(controls: SessionControls | undefined): SessionOption[] | undefined {
    if (controls === undefined || controls.modes.length === 0 ||
      !controls.options.some((c: ConfigControl) => c.category === ComposerControls.permissionCategory && c.type !== 'boolean')) {
      return undefined;
    }
    return controls.modes.slice(1);
  }

  /** The working mode in force (the pill), undefined at rest or without the split */
  static workingMode(controls: SessionControls | undefined): SessionOption | undefined {
    return ComposerControls.workingModes(controls)?.find((m: SessionOption) => m.id === controls?.modeId);
  }

  /**
   * composerControls.ts `sidekickClis`: the select's models by the CLI each runs in, which the host's select
   * (host_sidekick.rs) puts in `group`, the session's own CLI first. None and options without a CLI (the built-in
   * agent's own select) stay out, so fewer than two CLIs leave none to choose between
   */
  static sidekickClis(options: SessionOption[]): SidekickCli[] {
    const out: SidekickCli[] = [];
    for (const o of options) {
      const g = o.group;
      if (o.id === ComposerControls.noSidekick || g === undefined) {
        continue;
      }
      const cli = out.find((c: SidekickCli) => c.id === g.id);
      if (cli !== undefined) {
        cli.options.push(o);
      } else {
        const added: SidekickCli = { id: g.id, name: g.name, options: [o] };
        out.push(added);
      }
    }
    return out;
  }

  /**
   * composerControls.ts `sidekickChannel`: the channel a sidekick option comes through, its source; on the built-in
   * agent's own select (no CLI in `group`) the provider it puts in `description`. The host's select keeps the CLI's
   * name there, which is no channel
   */
  static sidekickChannel(o: SessionOption): string | undefined {
    if (o.source !== undefined) {
      return o.source.name;
    }
    return o.group === undefined ? o.description : undefined;
  }

  /**
   * composerControls.ts `sidekickFamilies`: group sidekick options into the same ModelFamily shape the lead's list
   * uses (collapsing embedded effort / Fast / 1M variants such as Factory's "Opus 5.5 Fast Mode" or Devin's "SWE-2 High Fast").
   */
  static sidekickFamilies(options: SessionOption[], cliId?: string): ModelFamily[] {
    const real = options.filter((o: SessionOption) => o.id !== ComposerControls.noSidekick);
    const normalized: SessionOption[] = [];
    for (const o of real) {
      if (cliId !== undefined) {
        const copy: SessionOption = { id: o.id, name: o.name, source: o.source, kind: o.kind };
        normalized.push(copy);
      } else if (o.source === undefined && o.group === undefined && o.description !== undefined && o.description.length > 0) {
        const copy: SessionOption = {
          id: o.id, name: o.name, source: { id: o.description, name: o.description, kind: 'custom' }, kind: o.kind
        };
        normalized.push(copy);
      } else {
        normalized.push(o);
      }
    }
    const families = ModelCatalog.group(normalized);
    if (cliId === undefined) {
      return families;
    }
    return families.map((f: ModelFamily) => {
      const copy: ModelFamily = {
        key: `${cliId}\0${f.key}`, name: f.name, brand: f.brand, source: f.source,
        sourceKind: f.sourceKind, variants: f.variants, efforts: f.efforts, hasFast: f.hasFast, hasLong: f.hasLong,
        fusion: f.fusion
      };
      return copy;
    });
  }

  /** Resolve the currently picked sidekick's family and variant across either a host-sidekick CLI list or a native select */
  static findSidekickFamily(options: SessionOption[], value: string | undefined): PickedSidekick | undefined {
    if (value === undefined || value.length === 0 || value === ComposerControls.noSidekick) {
      return undefined;
    }
    const clis = ComposerControls.sidekickClis(options);
    if (clis.length > 0) {
      for (const cli of clis) {
        const f = ComposerControls.sidekickFamilies(cli.options, cli.id)
          .find((fam: ModelFamily) => fam.variants.some((v: ModelVariant) => v.id === value));
        if (f !== undefined) {
          const v = f.variants.find((item: ModelVariant) => item.id === value);
          if (v !== undefined) {
            const hit: PickedSidekick = { family: f, variant: v, cli: cli };
            return hit;
          }
        }
      }
      return undefined;
    }
    const f = ComposerControls.sidekickFamilies(options)
      .find((fam: ModelFamily) => fam.variants.some((v: ModelVariant) => v.id === value));
    const v = f?.variants.find((item: ModelVariant) => item.id === value);
    if (f === undefined || v === undefined) {
      return undefined;
    }
    const hit: PickedSidekick = { family: f, variant: v };
    return hit;
  }

  /**
   * Families filed under their channel in first-seen order, those without one first unlabelled; undefined with fewer than
   * two channels, where the list stays flat (ModelPicker.tsx `bySource`)
   */
  static byChannelFamilies(families: ModelFamily[]): FamilyChannelGroup[] | undefined {
    const groups: FamilyChannelGroup[] = [];
    for (const f of families) {
      const channel = f.source !== undefined && !f.variants.some((v: ModelVariant) => v.id === f.source) ? f.source : '';
      const group = groups.find((g: FamilyChannelGroup) => g.channel === channel);
      if (group !== undefined) {
        group.families.push(f);
      } else {
        const added: FamilyChannelGroup = { channel: channel, families: [f] };
        groups.push(added);
      }
    }
    if (groups.filter((g: FamilyChannelGroup) => g.channel.length > 0).length < 2) {
      return undefined;
    }
    return groups.filter((g: FamilyChannelGroup) => g.channel.length === 0)
      .concat(groups.filter((g: FamilyChannelGroup) => g.channel.length > 0));
  }

  /**
   * Rows filed under their channel in first-seen order, those without one first unlabelled; undefined with fewer than
   * two channels, where the list stays flat (ModelPicker.tsx `bySource`)
   */
  static byChannel(options: SessionOption[]): ChannelGroup[] | undefined {
    const groups: ChannelGroup[] = [];
    for (const o of options) {
      const channel = ComposerControls.sidekickChannel(o) ?? '';
      const group = groups.find((g: ChannelGroup) => g.channel === channel);
      if (group !== undefined) {
        group.options.push(o);
      } else {
        const added: ChannelGroup = { channel: channel, options: [o] };
        groups.push(added);
      }
    }
    if (groups.filter((g: ChannelGroup) => g.channel.length > 0).length < 2) {
      return undefined;
    }
    return groups.filter((g: ChannelGroup) => g.channel.length === 0).concat(groups.filter((g: ChannelGroup) => g.channel.length > 0));
  }

  private static compact(value: string): string {
    return value.toLowerCase().replace(/reasoning|thinking|effort|level/g, '').replace(/[\s_-]/g, '');
  }

  /** Low / Medium / High / XHigh / Max / Ultra; the wire ids stay */
  static effortLabel(option: SessionOption): string {
    const canonical = (value: string): string | undefined => {
      const key = ComposerControls.compact(value);
      return key === 'extrahigh' ? 'XHigh' : EFFORT_LABELS.find((l: string) => l.toLowerCase() === key);
    };
    return canonical(option.id) ?? canonical(option.name) ?? option.name;
  }

  static isReasoning(c: ConfigControl): boolean {
    return c.category === 'thought_level' ||
      (c.category === undefined && /^(reasoning_effort|thought_level|thinking|thinking_level)$/.test(c.id));
  }

  /** Kimi folds thinking on / off into the effort select */
  private static toggleSide(o: SessionOption): string | undefined {
    const id = ComposerControls.compact(o.id);
    const key = id.length === 0 ? ComposerControls.compact(o.name) : id;
    return key === 'on' || key === 'off' ? key : undefined;
  }

  /** Kimi keeps the previous model's leftover value: drop the stranger, keep the native set */
  private static nativeThoughtOptions(options: SessionOption[]): SessionOption[] {
    const toggles = options.filter((o: SessionOption) => ComposerControls.toggleSide(o) !== undefined);
    const efforts = options.filter((o: SessionOption) => ComposerControls.toggleSide(o) === undefined);
    const ons = toggles.filter((o: SessionOption) => ComposerControls.toggleSide(o) === 'on');
    if (toggles.some((o: SessionOption) => ComposerControls.toggleSide(o) === 'off')) {
      return options;
    }
    if (ons.length > 0 && efforts.length > 1) {
      return efforts;
    }
    if (ons.length > 0 && efforts.length === 1) {
      return ons;
    }
    return options;
  }

  static presentReasoning(c: ConfigControl): Reasoning {
    const native = ComposerControls.nativeThoughtOptions(c.options);
    const plain = native.filter((o: SessionOption) => ComposerControls.toggleSide(o) === undefined);
    const rank = (name: string): number => {
      const at = EFFORT_LABELS.indexOf(name);
      return at >= 0 ? at : EFFORT_LABELS.length;
    };
    const labelled: SessionOption[] = plain.map((o: SessionOption) => {
      const out: SessionOption = { id: o.id, name: ComposerControls.effortLabel(o) };
      return out;
    });
    const efforts = labelled.map((o: SessionOption, i: number) => i).sort((a: number, b: number): number => {
      const d = rank(labelled[a].name) - rank(labelled[b].name);
      return d !== 0 ? d : a - b;
    }).map((i: number) => labelled[i]);
    const offId = native.find((o: SessionOption) => ComposerControls.toggleSide(o) === 'off')?.id;
    const onId = native.find((o: SessionOption) => ComposerControls.toggleSide(o) === 'on')?.id;
    const value = native.some((o: SessionOption) => o.id === c.value) ? c.value
      : plain.find((o: SessionOption) => o.id === 'high')?.id ?? plain[0]?.id ?? onId ?? native[0]?.id;
    const out: Reasoning = { efforts: efforts, offId: offId, onId: onId, value: value, off: offId !== undefined && value === offId };
    return out;
  }

  static reasoningVisible(c: ConfigControl): boolean {
    const p = ComposerControls.presentReasoning(c);
    return p.efforts.length > 1 || p.offId !== undefined;
  }

  static isFast(c: ConfigControl): boolean {
    if (c.type === 'boolean') {
      return /(^|[\s_-])fast($|[\s_-])/i.test(c.id) || /^fast( mode)?$/i.test(c.name);
    }
    return c.options.length === 2 &&
      ['standard', 'fast'].every((id: string) => c.options.some((o: SessionOption) => o.id === id));
  }

  static fastOn(c: ConfigControl): boolean {
    return c.value === (c.type === 'boolean' ? 'true' : 'fast');
  }

  static fastValue(c: ConfigControl, on: boolean): string {
    return c.type === 'boolean' ? `${on}` : on ? 'fast' : 'standard';
  }

  static split(options: ConfigControl[]): ControlSplit {
    const s: ControlSplit = {
      models: [], reasoning: [], modelConfig: [], collaboration: [], permission: [], agent: [], sidekick: [], sidekickEffort: [],
      sidekickFast: []
    };
    for (const c of options) {
      const plain = c.type !== 'boolean';
      if (ComposerControls.isReasoning(c)) {
        s.reasoning.push(c);
      } else if (c.category === 'collaboration_mode' && plain) {
        s.collaboration.push(c);
      } else if (c.category === ComposerControls.permissionCategory && plain) {
        s.permission.push(c);
      } else if (plain && (c.category === 'agent' || (c.category === undefined && c.id === 'agent'))) {
        s.agent.push(c);
      } else if (c.category === ComposerControls.sidekickCategory && plain) {
        s.sidekick.push(c);
      } else if (c.category === ComposerControls.sidekickEffortCategory && plain) {
        s.sidekickEffort.push(c);
      } else if (c.category === ComposerControls.sidekickFastCategory) {
        s.sidekickFast.push(c);
      } else if (c.category === 'model_config' || c.category === 'speed') {
        s.modelConfig.push(c);
      } else if (plain && (c.category === 'model' ||
        (c.category === undefined && (c.id === 'model' || ModelCatalog.group(c.options).length < c.options.length)))) {
        s.models.push(c);
      }
    }
    return s;
  }
}

/** One switch whatever its source (an embedded Fast variant, a Fusion pair, a native control, Thinking) */
export class Toggle {
  on: boolean;
  disabled: boolean;
  set: (on: boolean) => void;

  constructor(on: boolean, disabled: boolean, set: (on: boolean) => void) {
    this.on = on;
    this.disabled = disabled;
    this.set = set;
  }
}

export interface EffortLevel {
  id: string;
  name: string;
}

/** The reasoning levels on offer, Fast beside them, and the Thinking switch where thinking can be turned off */
export class Effort {
  levels: EffortLevel[];
  value: string;
  pick: (id: string) => void;
  fast: Toggle | undefined;
  thinking: Toggle | undefined;

  constructor(levels: EffortLevel[], value: string, pick: (id: string) => void, fast: Toggle | undefined,
    thinking: Toggle | undefined) {
    this.levels = levels;
    this.value = value;
    this.pick = pick;
    this.fast = fast;
    this.thinking = thinking;
  }
}

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
 * What the composer's model chip offers, read from the session's controls (Goo.swift ModelChoices): the model
 * families, the reasoning levels with Fast, the modes, the working mode, the approval level, and the sidekick (the
 * built-in agent's own or the one the host gives another CLI's session, found by category; or a Fusion pair's two
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

/** modeIcons.ts `modeIcon`: a mode's glyph from its kind, id and name; first match wins */
export function modeIcon(mode: SessionOption): string {
  if (mode.kind === 'full_access') {
    return 'shieldOff';
  }
  if (mode.kind === 'auto_review') {
    return 'sparkles';
  }
  if (mode.id === 'auto_edit' || mode.id === 'acceptEdits') {
    return 'code';
  }
  const key = `${mode.id} ${mode.name}`.toLowerCase();
  if (/bypass|yolo|auto[\s-]?accept|full[\s-]?access|unsafe/.test(key)) {
    return 'shieldOff';
  }
  if (/debug/.test(key)) {
    return 'bug';
  }
  if (/plan/.test(key)) {
    return 'listTodo';
  }
  if (/\bask\b|chat|question/.test(key)) {
    return 'messageCircleQuestionMark';
  }
  if (/smart|\bauto\b/.test(key)) {
    return 'sparkles';
  }
  if (/code|edit|build/.test(key)) {
    return 'code';
  }
  if (/\bteam\b|multitask|orchestrat/.test(key)) {
    return 'users';
  }
  if (/agent|default|normal/.test(key)) {
    return 'infinity';
  }
  return 'circleDashed';
}

const MODE_SHORT: Map<string, string> = new Map<string, string>([
  ['agent', 'agent'], ['build', 'build'], ['plan', 'plan'], ['raw', 'raw'], ['team', 'team'],
  ['acceptEdits', 'autoEdit'], ['auto_edit', 'autoEdit'], ['auto', 'auto'], ['smart', 'smart'],
  ['bypassPermissions', 'full'], ['bypass', 'full'], ['yolo', 'full'], ['agent-full-access', 'full'],
  ['read-only', 'readOnly'], ['workspace-write', 'workspace'], ['ask', 'ask'], ['code', 'code'], ['accept-edits', 'code']
]);

/**
 * modeIcons.ts `modeLabel`: the short name of a mode the built-in integrations send, matched on the exact id, in the
 * app's language; anything unknown shows the agent's own name
 */
export function modeLabel(mode: SessionOption): string {
  if (mode.kind === 'full_access') {
    return t('composer.modeShort.full');
  }
  if (mode.kind === 'auto_review') {
    return t('composer.modeShort.auto');
  }
  // Every known agent's `default` asks before acting (modeIcons.ts)
  if (mode.id === 'default') {
    return t('composer.access.ask');
  }
  const short = MODE_SHORT.get(mode.id);
  return short !== undefined ? t(`composer.modeShort.${short}`) : mode.name;
}

/** A collaboration mode's name: Codex's `default` is its Build mode, not the permission default beside it */
export function workLabel(mode: SessionOption): string {
  return mode.id === 'default' ? t('composer.modeShort.build') : modeLabel(mode);
}

/**
 * modeIcons.ts `modeName`: a working mode where it is picked by name (the `/` list, the bar's pill), the bare name
 * without the bean labels' padding to one length; unknown modes keep the label
 */
export function modeName(mode: SessionOption): string {
  if (mode.id === 'plan' || mode.id === 'team' || mode.id === 'raw') {
    return t(`composer.modeName.${mode.id}`);
  }
  return modeLabel(mode);
}

/** modeIcons.ts `modeHint`: one line on what the mode does, else the agent's own description */
export function modeHint(mode: SessionOption): string | undefined {
  if (mode.id === 'plan' || mode.id === 'team' || mode.id === 'raw') {
    return t(`composer.modeHint.${mode.id}`);
  }
  return mode.description;
}

const ACCESS_ICONS: Map<string, string> = new Map<string, string>([
  ['ask', 'shieldEllipsis'], ['auto-edit', 'shieldCheck'], ['auto', 'shieldHalf'], ['full', 'shieldOff']
]);

/**
 * modeIcons.ts `accessIcon`: the approval levels (the built-in agent's, and `auto` from the engine's split of Claude's
 * modes) as one shield family, so the bean reads as a scale; other levels keep their mode glyph
 */
export function accessIcon(levelOption: SessionOption): string {
  return ACCESS_ICONS.get(levelOption.id) ?? modeIcon(levelOption);
}

/** The built-in agent's approval levels by id, any other level its short mode label */
export function accessLabel(levelOption: SessionOption): string {
  if (levelOption.id === 'ask') {
    return t('composer.access.ask');
  }
  if (levelOption.id === 'auto-edit') {
    return t('composer.access.autoEdit');
  }
  if (levelOption.id === 'full') {
    return t('composer.access.full');
  }
  return modeLabel(levelOption);
}
