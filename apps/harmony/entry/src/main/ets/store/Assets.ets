import { FIXTURES, I18N } from '../generated/Assets.generated';

// The iOS app's dictionaries and demo fixtures, compiled in by scripts/build.sh (generated/Assets.generated.ets).

/** A dictionary (`en`, `zh-CN`) or fixture (`fixtures/<name>`) as JSON text; undefined when the build has none */
export function readRaw(path: string): string | undefined {
  if (path.startsWith('i18n/')) {
    return I18N.get(path.slice('i18n/'.length).replace('.json', ''));
  }
  if (path.startsWith('fixtures/')) {
    return FIXTURES.get(path.slice('fixtures/'.length).replace('.json', ''));
  }
  return undefined;
}
