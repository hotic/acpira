import type { ModelFamily } from './models';

export const MODEL_PREVIEW_LIMIT = 10;

// Compare versions only within the same named series and source, never across vendors.
// Keep size/date suffixes in the series identity rather than treating them as releases.
function release(family: ModelFamily) {
  const match = /\d+(?:[.-]\d+)*/.exec(family.name);
  if (!match || /^\d{6,}$/.test(match[0]) || /^[bkmt]\b/i.test(family.name.slice(match.index + match[0].length))) {
    return { series: family.key, version: [] as number[] };
  }
  const series = (family.name.slice(0, match.index) + ' ' + family.name.slice(match.index + match[0].length))
    .toLowerCase().replace(/[\s._-]+/g, ' ').trim();
  return { series: JSON.stringify([family.sourceKind, family.source, series]), version: match[0].split(/[.-]/).map(Number) };
}

// Promote each series' newest release before older releases; preserve ACP order for unrelated series.
export function prioritizeModels(families: ModelFamily[]): ModelFamily[] {
  const series = new Map<string, { family: ModelFamily; version: number[] }[]>();
  for (const family of families) {
    const parsed = release(family);
    const bucket = series.get(parsed.series) ?? [];
    bucket.push({ family, version: parsed.version });
    series.set(parsed.series, bucket);
  }
  for (const bucket of series.values()) bucket.sort((left, right) => {
    for (let index = 0; index < Math.max(left.version.length, right.version.length); index++) {
      const difference = (right.version[index] ?? 0) - (left.version[index] ?? 0);
      if (difference) return difference;
    }
    return 0;
  });
  return [
    ...[...series.values()].map(bucket => bucket[0]!.family),
    ...[...series.values()].flatMap(bucket => bucket.slice(1).map(item => item.family)),
  ];
}

export function filterModels(families: ModelFamily[], query: string): ModelFamily[] {
  const terms = query.toLowerCase().trim().split(/\s+/).filter(Boolean);
  return families.filter(family => {
    const text = [family.name, family.source, ...family.variants.flatMap(variant => [variant.id, variant.name])].join(' ').toLowerCase();
    return terms.every(term => text.includes(term));
  });
}

// One bulk preference change includes collapsed/search-hidden rows and retains absent catalog entries.
export function setModelsVisible(families: ModelFamily[], hidden: string[], visible: boolean): string[] {
  const next = new Set(hidden);
  for (const family of families) {
    next.delete(family.name);
    if (visible) next.delete(family.key); else next.add(family.key);
  }
  return [...next];
}
