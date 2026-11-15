import { useEffect, useState, type ReactNode } from 'react';
import { X } from 'lucide-react';
import type { AgentTurn, TurnStop } from '@acpira/shared/transcript';
import type { MsgKey } from '@acpira/shared/i18n/core';
import { isContextLengthError } from '@acpira/shared/turnErrors';
import { Card } from '../ui/Card';
import { Button, IconButton } from '../ui/Button';
import { t } from '../i18n';

export interface AlertProps {
  turn: AgentTurn;
  // Carry the failed turn on, or send an empty attempt again; a second failure reconnects first (error) / ask the agent to carry on (limits)
  onRetry: () => void;
  onContinue: () => void;
  onDismiss: () => void;
  onCompact?: () => void;
  // AIR sessionFailure 'new_session' action
  onNewSession?: () => void;
}

// Shared by the dock's alert cards (this one and chat/Notice): the glyph sits in a Row-sized lead slot so the text below
// starts at the title's column (pl-indent), and actions are the small in-card buttons, so the title row stays one text
// line tall instead of growing to a full control
export const DOCK_BUTTON = 'h-ctl-sm px-2 text-3';

// A turn stopped short (the same card Cursor pins above its composer, in our own tones): one row holds a colorless glyph, what happened,
// the copyable detail, and the action — retry for an error, carry on for a limit, nothing for a refusal —
// with ✕ hiding the card (the transcript keeps the row); the agent's words sit below in small type when there are any
export function Alert({ turn, onRetry, onContinue, onDismiss, onCompact, onNewSession }: AlertProps) {
  const stop = turn.stop as ShortStop;
  const err = turn.error;
  const contextTooLong = stop === 'error' && isContextLengthError(err);
  const detail = [err?.code !== undefined ? String(err.code) : '', err?.kind ?? ''].filter(Boolean).join(t('common.metaSep'));
  const copyable = [err?.message, detail].filter(Boolean).join('\n');
  const message = contextTooLong ? t(onCompact ? 'alert.contextLength.text' : 'alert.contextLength.unsupported')
    : stop === 'error' ? err?.message || t('alert.error.unknown') : TEXT[stop] && t(TEXT[stop]);
  const retry = stop === 'error' && !contextTooLong && (err?.actions === undefined || err.actions.includes('retry'));
  return (
    <DockCard
      role="alert"
      title={t(contextTooLong ? 'alert.contextLength.title' : TITLE[stop])}
      detail={copyable && <CopyDetail text={copyable} label={detail} />}
      text={message}
      // An AIR sessionFailure names its own remedies; 'login' has no button here because the Notice card owns sign-in.
      // Retry is the only remedy otherwise: it continues where the turn stopped and reconnects by itself on a repeat failure
      alt={stop === 'error' && !contextTooLong && err?.actions?.includes('new_session') === true && onNewSession
        && <Button variant="secondary" className={DOCK_BUTTON} onClick={onNewSession}>{t('notice.continueNew')}</Button>}
      actions={<>
        {retry && <Button variant="primary" className={DOCK_BUTTON} title={t('alert.retryHint')} onClick={onRetry}>{t('common.retry')}</Button>}
        {(stop === 'max_tokens' || stop === 'max_turn_requests') && <Button variant="primary" className={DOCK_BUTTON} onClick={onContinue}>{t('alert.continue')}</Button>}
        {contextTooLong && onCompact && <Button variant="primary" className={DOCK_BUTTON} onClick={onCompact}>{t('alert.contextLength.compact')}</Button>}
      </>}
      trail={<IconButton size="sm" aria-label={t('common.close')} onClick={onDismiss}><X strokeWidth={1.5} /></IconButton>}
    />
  );
}

// The skeleton every dock card shares (this alert, chat/Notice): a title with its one line of text as one block, the
// actions beside that block and centred on it, the primary last, and a small icon control (dismiss, re-check) at the
// end. `alt` is the way around the primary (a menu or a second button): with one, a card below the notice tier takes
// the question card's shape, the actions in a footer under a hairline with `alt` at its start (styles/chat.css).
// No glyph leads the title: beside two lines of text it read as a stray mark
export function DockCard({ title, detail, text, textRole, alt, actions, trail, role, busy }: {
  title: string; detail?: ReactNode; text?: ReactNode; textRole?: 'status'; alt?: ReactNode; actions?: ReactNode; trail?: ReactNode; role?: 'alert'; busy?: boolean;
}) {
  return (
    <div className="px-dock">
      <Card role={role} className="dock-card">
        <div className="dock-card-body" data-foot={alt ? '' : undefined}>
          <div className="flex min-w-0 flex-1 flex-col gap-0.5">
            <div className="flex min-w-0 items-center gap-gap">
              <span className="min-w-0 truncate text-2 font-medium text-fg-1" title={title}>{title}</span>
              {detail}
            </div>
            {text && <p role={textRole} className="m-0 whitespace-pre-wrap text-3 text-fg-3 [overflow-wrap:anywhere]">{text}</p>}
          </div>
          <fieldset disabled={busy} aria-busy={busy} className="dock-card-actions disabled:opacity-60">
            {alt && <span className="dock-card-alt flex min-w-0">{alt}</span>}
            {actions}
          </fieldset>
          {trail && <span className="dock-card-trail flex">{trail}</span>}
        </div>
      </Card>
    </div>
  );
}

type ShortStop = Exclude<TurnStop, 'end_turn' | 'cancelled'>;

const TITLE: Record<ShortStop, MsgKey> = {
  error: 'alert.error.title',
  refusal: 'alert.refusal.title',
  max_tokens: 'alert.maxTokens.title',
  max_turn_requests: 'alert.maxTurns.title',
};

const TEXT: Record<ShortStop, MsgKey | undefined> = {
  error: undefined,
  refusal: 'alert.refusal.text',
  max_tokens: 'alert.maxTokens.text',
  max_turn_requests: 'alert.maxTurns.text',
};

// Cursor's "Copy Request (id)": a faint text button that copies the whole detail and confirms for a moment; the visible label is the code · kind line when there is one,
// the detail itself its tooltip
export function CopyDetail({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const h = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(h);
  }, [copied]);
  return (
    <button
      type="button"
      title={text}
      className="min-w-0 truncate rounded-md px-1.5 py-0.5 text-3 text-fg-3 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1"
      onClick={() => { void navigator.clipboard.writeText(text).then(() => setCopied(true)); }}
    >
      {copied ? t('alert.copied') : label ? t('alert.copyWith', { detail: label }) : t('alert.copy')}
    </button>
  );
}

// Whether this turn is one the card should stand up for: it ended short and the session is otherwise usable (a login problem has the Notice)
export function isShortStop(turn: AgentTurn | undefined): turn is AgentTurn & { stop: ShortStop } {
  return !!turn?.stop && turn.stop !== 'end_turn' && turn.stop !== 'cancelled';
}
