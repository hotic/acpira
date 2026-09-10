import type { AgentTurn } from '@shared/transcript';
import { translate, type Locale } from '@shared/i18n';

// An empty ACP completion is a receipt, not proof that a command took effect.
// Errors/cancellation always win; actual prose and tool results stand on their own.
export function turnOutcome(turn: AgentTurn, locale: Locale): string | undefined {
  const t = (key: Parameters<typeof translate>[1], params?: Parameters<typeof translate>[2]) => translate(locale, key, params);
  switch (turn.stop) {
    case 'error': return t('turns.stop.error');
    case 'refusal': return t('turns.stop.refusal');
    case 'max_tokens': return t('turns.stop.maxTokens');
    case 'max_turn_requests': return t('turns.stop.maxTurns');
    case 'cancelled': return t('turns.stop.cancelled');
    default: {
      if (turn.stop !== 'end_turn' || turn.blocks.some(b => b.type !== 'text' || b.markdown.trim())) return undefined;
      if (!turn.command) return t('turns.stop.empty');
      const { mode, options } = turn.command;
      const changes = [mode ? t('turns.command.mode', { mode }) : '',
        ...(options ?? []).map(o => t('turns.command.option', o))].filter(Boolean);
      return changes.length ? changes.join(t('common.listSep')) : t('turns.command.empty');
    }
  }
}
