import { t } from '../i18n';

// Command row run time. DOM-free so the host tsconfig can type-check its test.

/**
 * A command's run time for the row's trailing slot. Finished commands keep a tenth of a second below ten seconds, one
 * tenth as the floor; `whole` is the live clock, which ticks in whole seconds.
 */
export function commandDuration(ms: number, whole = false): string {
  const clamped = Math.max(0, ms);
  // Rounded first, so 9.96 s carries into the whole-second branch as "10s" instead of "10.0s"
  const tenths = Math.round(Math.max(100, clamped) / 100);
  if (!whole && tenths < 100) return t('turns.elapsed.s', { s: (tenths / 10).toFixed(1) });
  const seconds = whole ? Math.floor(clamped / 1000) : Math.round(clamped / 1000);
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return m ? (s ? t('turns.elapsed.ms', { m, s }) : t('turns.elapsed.m', { m })) : t('turns.elapsed.s', { s });
}
