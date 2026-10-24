import { t } from '../i18n';

// Command row run time. DOM-free so the host tsconfig can type-check its test.

/**
 * A command's run time for the row's trailing slot. Finished commands keep a tenth of a second below ten seconds;
 * `whole` is the live clock, which ticks in whole seconds.
 */
export function commandDuration(ms: number, whole = false): string {
  const clamped = Math.max(0, ms);
  if (!whole && clamped < 10_000) return t('turns.elapsed.s', { s: clamped < 100 ? '<0.1' : (clamped / 1000).toFixed(1) });
  const seconds = whole ? Math.floor(clamped / 1000) : Math.round(clamped / 1000);
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return m ? (s ? t('turns.elapsed.ms', { m, s }) : t('turns.elapsed.m', { m })) : t('turns.elapsed.s', { s });
}
