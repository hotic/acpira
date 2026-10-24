import type { AgentTurn, NoticeBlock } from '@shared/transcript';
import { translate, type Locale } from '@shared/i18n';

const NO_NOTICES: NoticeBlock[] = [];

// A turn that ended on its own session/prompt error (no AIR failure id behind it) already names the cause, and the
// Notice / Alert cards own the remedy. Error-severity notices on that turn are earlier symptoms of the same failure
// (claude-agent-acp: "The connection to Claude was lost." with `new_session`, then -32000 Authentication required),
// so they join the outcome row as detail lines instead of standing as separate warnings with remedies of their own
export function absorbedNotices(turn: AgentTurn): NoticeBlock[] {
  if (turn.stop !== 'error' || !turn.error || turn.error.failureId !== undefined) return NO_NOTICES;
  const found = turn.blocks.filter((b): b is NoticeBlock => b.type === 'notice' && b.severity === 'error');
  return found.length ? found : NO_NOTICES;
}

// Copy / fork / stats only mean something for a turn that produced something beyond failure notices and goal milestones
export function hasTurnContent(turn: AgentTurn): boolean {
  return turn.blocks.some(b => b.type !== 'notice' && b.type !== 'goal');
}

// An empty ACP completion is a receipt, not proof that a command took effect.
// Errors/cancellation always win; actual prose and tool results stand on their own.
export function turnOutcome(turn: AgentTurn, locale: Locale): string | undefined {
  const t = (key: Parameters<typeof translate>[1], params?: Parameters<typeof translate>[2]) => translate(locale, key, params);
  if (turn.observation === 'unknown') return t('chatgpt.unknownTurn');
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
