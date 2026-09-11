import { useEffect, useState } from 'react';
import { TriangleAlert, X } from 'lucide-react';
import type { AgentTurn, TurnStop } from '@shared/transcript';
import type { MsgKey } from '@shared/i18n';
import { Card } from '../ui/Card';
import { Button, IconButton } from '../ui/Button';
import { t } from '../i18n';

export interface AlertProps {
  turn: AgentTurn;
  // Send the same prompt again (error) / rebuild the connection and resume the session (error) / ask the agent to carry on (limits)
  onRetry: () => void;
  onReconnect: () => void;
  onContinue: () => void;
  onDismiss: () => void;
}

// A turn stopped short (the same card Cursor pins above its composer, in our own tones): one row holds a colorless glyph, what happened,
// the copyable detail, and the action — send it again or reconnect and resume for an error, carry on for a limit, nothing for a refusal —
// with ✕ hiding the card (the transcript keeps the row); the agent's words sit below in small type when there are any
export function Alert({ turn, onRetry, onReconnect, onContinue, onDismiss }: AlertProps) {
  const stop = turn.stop as ShortStop;
  const err = turn.error;
  const detail = [err?.code !== undefined ? String(err.code) : '', err?.kind ?? ''].filter(Boolean).join(' · ');
  const copyable = [err?.message, detail].filter(Boolean).join('\n');
  const message = stop === 'error' ? err?.message || t('alert.error.unknown') : TEXT[stop] && t(TEXT[stop]);
  return (
    <div className="px-page pt-2">
      <Card role="alert" className="flex flex-col gap-1.5 px-pad py-2.5">
        <div className="flex items-center gap-2">
          <TriangleAlert className="size-icon shrink-0 text-fg-3" strokeWidth={1.75} />
          <span className="shrink-0 text-2 font-medium text-fg-1">{t(TITLE[stop])}</span>
          {copyable && <CopyDetail text={copyable} label={detail} />}
          <span className="flex-1" />
          {stop === 'error' && <Button variant="secondary" title={t('alert.reconnectHint')} onClick={onReconnect}>{t('alert.reconnect')}</Button>}
          {stop === 'error' && <Button variant="primary" onClick={onRetry}>{t('common.retry')}</Button>}
          {(stop === 'max_tokens' || stop === 'max_turn_requests') && <Button variant="primary" onClick={onContinue}>{t('alert.continue')}</Button>}
          <IconButton aria-label={t('common.close')} onClick={onDismiss} className="-my-1 -mr-1.5"><X strokeWidth={1.5} /></IconButton>
        </div>
        {message && <p className="m-0 whitespace-pre-wrap text-3 text-fg-2 [overflow-wrap:anywhere]">{message}</p>}
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

// Cursor's "Copy Request (id)": a faint text button that copies the whole detail and confirms for a moment; the visible label is the code · kind line when there is one
function CopyDetail({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const h = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(h);
  }, [copied]);
  return (
    <button
      type="button"
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
