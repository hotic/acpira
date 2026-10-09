import type { Turn } from '@shared/transcript';
import { getLocale, t } from '../i18n';

// Goal strip / milestone texts and the "Set as goal" prompt pick. DOM-free so the host tsconfig can type-check its test.

/** Elapsed goal time: "45s", "6m 12s", "2h 05m" */
export function goalDuration(seconds: number): string {
  const whole = Math.max(0, Math.round(seconds));
  const h = Math.floor(whole / 3600);
  const m = Math.floor(whole / 60) % 60;
  const s = whole % 60;
  if (h) return t('goal.elapsed.hm', { h, m: String(m).padStart(2, '0') });
  if (m) return s ? t('turns.elapsed.ms', { m, s }) : t('turns.elapsed.m', { m });
  return t('turns.elapsed.s', { s });
}

const tokens = (n: number) => new Intl.NumberFormat(getLocale(), { notation: 'compact', maximumFractionDigits: 1 }).format(n);

/** Token spend, against the budget when the agent set one ("184K / 500K tokens") */
export function goalSpend(goal: { tokensUsed?: number; tokenBudget?: number }): string | undefined {
  if (goal.tokensUsed === undefined) return undefined;
  return goal.tokenBudget
    ? t('goal.tokensOf', { used: tokens(goal.tokensUsed), budget: tokens(goal.tokenBudget) })
    : t('goal.tokens', { used: tokens(goal.tokensUsed) });
}

/**
 * The turn "Set as goal" hangs under: the newest prompt the user typed (automatic turns are skipped), unless that one
 * is a slash command or carries no text
 */
export function lastTypedPrompt(turns: Turn[]): number | undefined {
  let i = turns.length - 1;
  while (i >= 0) {
    const turn = turns[i]!;
    if (turn.role === 'user' && !turn.auto) break;
    i--;
  }
  const turn = turns[i];
  if (turn?.role !== 'user' || !turn.text.trim() || turn.text.trimStart().startsWith('/')) return undefined;
  return i;
}
