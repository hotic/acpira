import type { SessionOption } from './transcript';

// Reconstructing the model list: Devin flattens the cartesian product of "model × reasoning effort × Fast × 1M" into 210 flat options
// (no grouping, and no separate thought_level configOption; _meta only has supportsImages), so the structure can only be recovered
// from the names.
// Name pattern: <family> [effort] [Thinking] [Fast] [1M]; see parseModelName for how effort words combine with Thinking.
// When no structure can be parsed (Grok's 4 monolithic names), groupModels yields family count = option count, and callers just render flat as before.
// Devin's Fusion pairs ("Fusion (<lead> + <sidekick>)", 210 more flat options) collapse into one family whose variants carry
// the lead / sidekick dimensions; see parseFusionName

export interface ModelVariant {
  // Option value of the configOption; required when calling set_config_option
  id: string;
  name: string;
  // Reasoning effort label: None / Minimal / Low / Medium / High / XHigh / Max / Thinking; '' means this dimension is absent (Standard)
  effort: string;
  fast: boolean;
  // 1M context variant
  long: boolean;
  // Fusion only: the lead model family (effort / fast above describe the lead) and the sidekick label ("SWE-2 High")
  lead?: string;
  sidekick?: string;
}

export interface ModelFamily {
  // Namespaced identity keeps equal display names from different providers separate.
  key: string;
  name: string;
  // Vendor mark key resolved from the options' wire ids (see optionBrand); undefined when unbranded
  brand?: string;
  // Agent-supplied blurb, when every option of the family carries the same one (Devin's Fusion / Adaptive)
  description?: string;
  source?: string;
  sourceKind?: 'official' | 'custom';
  variants: ModelVariant[];
  // Effort labels sorted by strength (deduplicated)
  efforts: string[];
  hasFast: boolean;
  hasLong: boolean;
  // Fusion only: lead families and sidekick labels in first-seen order
  fusion?: { leads: string[]; sidekicks: string[] };
}

export const FUSION = 'Fusion';
const FUSION_NAME = /^Fusion\s*\((.+?)\s+\+\s+(.+)\)$/i;

// "Fusion (GPT-6 Astra High Thinking Fast + GPT-5.6 Luna High Thinking Fast)" → lead GPT-6 Astra at High, sidekick "GPT-5.6 Luna High", fast.
// Fast is one pair-level switch on the wire (`-fast-` after the lead, `-priority` on the sidekick): the name marks it on whichever
// side supports it, so a Fable lead (no Fast) still reads Fast from its sidekick. The sidekick's own effort stays in its label
export function parseFusionName(name: string): (Omit<ModelVariant, 'id' | 'name'> & { family: string }) | undefined {
  const m = FUSION_NAME.exec(name.trim());
  if (!m) return;
  const lead = parseModelName(m[1]!), sk = parseModelName(m[2]!);
  return { family: FUSION, effort: lead.effort, fast: lead.fast || sk.fast, long: lead.long, lead: lead.family, sidekick: [sk.family, sk.effort].filter(Boolean).join(' ') };
}

// Effort word → unified label (both X-High and XHigh spellings occur)
const LEVEL: Record<string, string> = {
  none: 'None', minimal: 'Minimal', low: 'Low', medium: 'Medium', high: 'High', xhigh: 'XHigh', 'x-high': 'XHigh', max: 'Max',
};
const EFFORT_ORDER = ['', 'Thinking', 'None', 'Minimal', 'Low', 'Medium', 'High', 'XHigh', 'Max'];

export function parseModelName(name: string): Omit<ModelVariant, 'id' | 'name'> & { family: string } {
  const fusion = parseFusionName(name);
  if (fusion) return fusion;
  const t = name.trim().split(/\s+/);
  let fast = false, long = false, thinking = false, effort = '';
  // Strip suffixes first: Fast / 1M may both be present, in either order
  for (;;) {
    const last = t[t.length - 1]?.toLowerCase();
    if (t.length > 1 && last === 'fast') { fast = true; t.pop(); }
    else if (t.length > 1 && last === '1m') { long = true; t.pop(); }
    else break;
  }
  if (t.length > 1 && t[t.length - 1]!.toLowerCase() === 'thinking') { thinking = true; t.pop(); }
  const last = t[t.length - 1]!.toLowerCase();
  if (t.length > 1 && last in LEVEL) { effort = LEVEL[last]!; t.pop(); }
  else if (t.length > 1 && last === 'no' && thinking) { effort = 'None'; thinking = false; t.pop(); }
  // For names like "Claude Opus 4.6 Thinking" with no effort word, just a toggle, Thinking itself counts as a level
  if (!effort && thinking) effort = 'Thinking';
  return { family: t.join(' '), effort, fast, long };
}

export function groupModels(options: SessionOption[]): ModelFamily[] {
  const map = new Map<string, ModelFamily>();
  // With no provider/group metadata, equal parameter tuples are ambiguous, not interchangeable.
  const tuples = new Set<string>();
  const ambiguous = new Set<string>();
  for (const o of options) {
    const p = parseModelName(o.name);
    const base = JSON.stringify([o.source?.id ?? o.group?.id, p.family]);
    const tuple = JSON.stringify([base, p.effort, p.fast, p.long, p.lead, p.sidekick]);
    if (tuples.has(tuple)) ambiguous.add(base);
    tuples.add(tuple);
  }
  for (const o of options) {
    const p = parseModelName(o.name);
    const namespace = o.source?.id ?? o.group?.id;
    const separate = ambiguous.has(JSON.stringify([namespace, p.family]));
    const source = o.source?.name ?? o.group?.name ?? (separate ? o.id : undefined);
    const key = separate ? JSON.stringify([namespace ?? null, p.family, o.id]) : namespace ? JSON.stringify([namespace, p.family]) : p.family;
    let f = map.get(key);
    if (!f) { f = { key, name: p.family, description: o.description, source, sourceKind: o.source?.kind, variants: [], efforts: [], hasFast: false, hasLong: false }; map.set(key, f); }
    // Fusion ids start with the lead's vendor ("fusion-claude-…"); the pair is Devin's own routing, like Adaptive
    f.brand ??= p.lead ? 'devin' : optionBrand(o);
    if (f.description !== o.description) f.description = undefined;
    if (p.lead) {
      f.fusion ??= { leads: [], sidekicks: [] };
      if (!f.fusion.leads.includes(p.lead)) f.fusion.leads.push(p.lead);
      if (p.sidekick && !f.fusion.sidekicks.includes(p.sidekick)) f.fusion.sidekicks.push(p.sidekick);
    }
    f.variants.push({ id: o.id, name: o.name, effort: p.effort, fast: p.fast, long: p.long, ...(p.lead ? { lead: p.lead, sidekick: p.sidekick } : {}) });
  }
  const rank = (e: string) => { const i = EFFORT_ORDER.indexOf(e); return i < 0 ? EFFORT_ORDER.length : i; };
  for (const f of map.values()) {
    const leadRank = (v: ModelVariant) => f.fusion?.leads.indexOf(v.lead ?? '') ?? 0;
    const skRank = (v: ModelVariant) => f.fusion?.sidekicks.indexOf(v.sidekick ?? '') ?? 0;
    f.variants.sort((a, b) => leadRank(a) - leadRank(b) || rank(a.effort) - rank(b.effort) || skRank(a) - skRank(b) || Number(a.fast) - Number(b.fast) || Number(a.long) - Number(b.long));
    f.efforts = [...new Set(f.variants.map(v => v.effort))].sort((a, b) => rank(a) - rank(b));
    f.hasFast = f.variants.some(v => v.fast);
    f.hasLong = f.variants.some(v => v.long);
  }
  return [...map.values()];
}

// The pair a Fusion variant stands for: lead × effort × sidekick × fast. Exact match first, then the nearest offered pair:
// drop Fast → keep lead + effort and take the first sidekick → keep lead and take the nearest effort → any variant of that lead
export function findFusionVariant(f: ModelFamily, want: { lead: string; effort: string; sidekick?: string; fast: boolean }): ModelVariant | undefined {
  const of = (lead: string) => f.variants.filter(v => v.lead === lead);
  const pool = of(want.lead).length ? of(want.lead) : f.variants;
  const hit = (effort: string, sidekick: string | undefined, fast: boolean) => pool.find(v => v.effort === effort && (sidekick === undefined || v.sidekick === sidekick) && v.fast === fast);
  const at = (effort: string) => hit(effort, want.sidekick, want.fast) ?? hit(effort, want.sidekick, false) ?? hit(effort, undefined, want.fast) ?? hit(effort, undefined, false);
  return at(want.effort) ?? nearestEffort(pool, want.effort).map(at).find(Boolean) ?? pool[0];
}

// Efforts of the pool ordered by distance from the wanted one (ties: the stronger first)
function nearestEffort(pool: ModelVariant[], effort: string): string[] {
  const rank = (e: string) => { const i = EFFORT_ORDER.indexOf(e); return i < 0 ? EFFORT_ORDER.length : i; };
  return [...new Set(pool.map(v => v.effort))].filter(e => e !== effort).sort((a, b) => Math.abs(rank(a) - rank(effort)) - Math.abs(rank(b) - rank(effort)) || rank(b) - rank(a));
}

// "GPT-6 Astra High + SWE-2 High Fast": the full pair for tooltips and settings rows; Fast belongs to the pair, so it trails
export function fusionLabel(v: ModelVariant): string {
  return [[v.lead, v.effort].filter(Boolean).join(' '), [v.sidekick, v.fast && 'Fast'].filter(Boolean).join(' ')].filter(Boolean).join(' + ');
}

export const familyHidden = (f: ModelFamily, hidden: string[]): boolean => hidden.includes(f.key) || hidden.includes(f.name);

// Expand legacy name-only preferences before toggling one source, preserving its siblings.
export function setFamilyVisible(options: SessionOption[], hidden: string[], key: string, show: boolean): string[] {
  const families = groupModels(options);
  const next = new Set(hidden);
  for (const f of families) {
    if (hidden.includes(f.name)) { next.delete(f.name); next.add(f.key); }
  }
  if (show) next.delete(key); else next.add(key);
  return [...next];
}

// Drop the options whose family is hidden; the current value always stays reachable, and a list that would hide everything shows everything
export function visibleOptions(options: SessionOption[], hidden: string[] | undefined, current?: string): SessionOption[] {
  if (!hidden?.length) return options;
  const hiddenIds = new Set(groupModels(options).filter(f => familyHidden(f, hidden)).flatMap(f => f.variants.map(v => v.id)));
  const kept = options.filter(o => o.id === current || !hiddenIds.has(o.id));
  return kept.length ? kept : options;
}

// Find a variant: exact match first; otherwise drop 1M → Fast → both in turn, finally fall back to the first variant at that effort
export function findVariant(f: ModelFamily, effort: string, fast: boolean, long: boolean): ModelVariant | undefined {
  const hit = (fa: boolean, lo: boolean) => f.variants.find(v => v.effort === effort && v.fast === fa && v.long === lo);
  return hit(fast, long) ?? hit(fast, false) ?? hit(false, long) ?? hit(false, false) ?? f.variants.find(v => v.effort === effort);
}

// Text on the params chip: "Max Fast 1M"; when the family has an effort dimension but this variant has no effort word, call it Standard; also Standard when there's nothing at all.
// Callers that show this in the UI pass a translated `standard`; tests and host-side labels keep the English default
export function variantLabel(v: ModelVariant, f: ModelFamily, labels?: { standard: string }): string {
  if (v.lead) return fusionLabel(v);
  const standard = labels?.standard ?? 'Standard';
  const parts = [v.effort || (f.efforts.length > 1 ? standard : ''), v.fast && 'Fast', v.long && '1M'].filter(Boolean);
  return parts.join(' ') || standard;
}

// Model → vendor brand key (see webview chat/marks.tsx for the matching logos). Purely heuristic: case-insensitive keyword
// rules on word boundaries, first hit wins. Ids are slugs like "asgard/kimi-k3", so / and - count as word boundaries.
// "Adaptive" / "Fusion" map to devin because they are Devin's own routing models; no match yields undefined and the renderer falls
// back to an initial-letter tile
const BRAND: [RegExp, string][] = [
  [/\bclaude\b/, 'claude'],
  [/\bglm\b|\bzhipu\b|\bchatglm\b/, 'zhipu'],
  [/\bkimi\b|\bmoonshot\b/, 'kimi'],
  // Moonshot's bare K-series names (K2.7 Coding, K3, K3-256k) carry no "kimi" keyword — a last resort for
  // options whose wire id is also opaque (e.g. a gateway alias like "asgard")
  [/\bk\d+(\.\d+)?\b/, 'kimi'],
  [/\bswe\b|\bwindsurf\b/, 'windsurf'],
  [/\badaptive\b|\bdevin\b|\bfusion\b/, 'devin'],
  [/\bgpt\b|\bopenai\b|\bcodex\b/, 'openai'],
  [/\bgemini\b|\bgemma\b/, 'gemini'],
  [/\bgrok\b/, 'grok'],
  [/\bdeepseek\b/, 'deepseek'],
  [/\bqwen\b|\btongyi\b/, 'qwen'],
  [/\bmistral\b|\bmixtral\b|\bcodestral\b|\bdevstral\b/, 'mistral'],
  [/\bllama\b/, 'llama'],
];

export function modelBrand(family: string): string | undefined {
  const t = family.toLowerCase();
  for (const [re, brand] of BRAND) if (re.test(t)) return brand;
  return undefined;
}

// Brand of a session option: the wire id is the real model identity ("kimi-code/k3", "asgard/kimi-k3") while the
// display name is only a label ("K3"), so the id is checked first and the name is just a fallback for opaque ids
export function optionBrand(o: Pick<SessionOption, 'id' | 'name'>): string | undefined {
  return modelBrand(o.id) ?? modelBrand(o.name);
}
