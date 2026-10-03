import type { AgentTurn, QueuedPrompt, SessionStatus, Turn, UserTurn } from '@shared/transcript';

export interface HeldView {
  turns: Turn[];
  running: boolean;
  queued?: QueuedPrompt[];
}

/**
 * A prompt queued while no turn runs is only held back by the session itself: the CLI still starting, the remembered
 * model / effort / mode being replayed into a new session, an account switch. It goes out as soon as that ends, so it
 * reads as sent: the head entry shows as the user's message with a working reply below it instead of a queued row over
 * an empty transcript. The host gives the sent user turn the entry's id, so the bubble keeps its key when it lands.
 * Entries behind the head stay queued rows; they wait for the head's turn like any follow-up.
 */
export function heldPrompt(turns: Turn[], running: boolean, status: SessionStatus, queued: QueuedPrompt[] | undefined): HeldView {
  const head = queued?.[0];
  if (running || !head || head.sending || (status !== 'starting' && status !== 'ready')) return { turns, running, queued };
  const user: UserTurn = {
    role: 'user',
    id: head.id,
    text: head.text,
    ...(head.attachments.length ? { attachments: head.attachments } : {}),
  };
  const reply: AgentTurn = { role: 'agent', blocks: [] };
  const rest = queued.slice(1);
  return { turns: [...turns, user, reply], running: true, queued: rest.length ? rest : undefined };
}
