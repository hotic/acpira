import { useState } from 'react';
import { ArrowUp, ListEnd, Merge, Pencil, Trash2 } from 'lucide-react';
import type { Draft, QueuedPrompt, SessionControls } from '@shared/transcript';
import { t } from '../i18n';
import { IconButton } from '../ui/Button';
import { Row, RowTarget } from '../ui/Row';
import { cn } from '../ui/cn';
import { surfaceVariants } from '../ui/Surface';
import { Composer, type ComposerProps } from './Composer';
import { AttachmentTiles, EditAttachments } from './Attachments';

export interface QueueHandlers {
  remove: (id: string) => void;
  sendNow?: (id: string) => void;
  // Present while the turn runs on an agent that can steer (and the setting is on): the send button steers instead
  steer?: (id: string) => void;
  edit: (id: string, text: string, retainedAttachments: number[], attachments: Draft[]) => void;
}

// Prompts waiting for the running turn, stacked right above the composer the way Cursor shows them: one row each — queue glyph, the attachments
// as small tiles, the first line of the text — with a pencil and a bin at the end. The pencil (or the text) swaps the row for an inline editor;
// saving replaces the entry in place, so it keeps its position in the queue
export function Queue({ items, composer, blobUrl, on }: { items: QueuedPrompt[]; composer: ComposerProps; blobUrl?: (blob: string) => string; on?: QueueHandlers }) {
  const [editing, setEditing] = useState<string>();
  const sending = items.some(item => item.sending);
  return (
    <div className="flex flex-col gap-(--dock-gap) px-page">
      {items.map(item => (
        editing === item.id && on
          ? <QueuedEditor key={item.id} item={item} composer={composer} blobUrl={blobUrl} onSave={(text, retained, drafts) => on.edit(item.id, text, retained, drafts)} onClose={() => setEditing(undefined)} />
          : <QueuedRow key={item.id} item={item} blobUrl={blobUrl} sending={sending} disabled={composer.disabled}
              onEdit={on && (() => setEditing(item.id))} onRemove={on && (() => on.remove(item.id))}
              onSendNow={on?.sendNow && (() => on.sendNow!(item.id))} onSteer={on?.steer && (() => on.steer!(item.id))} />
      ))}
    </div>
  );
}

function QueuedRow({ item, blobUrl, sending, disabled, onEdit, onRemove, onSendNow, onSteer }: {
  item: QueuedPrompt; blobUrl?: (blob: string) => string; sending: boolean; disabled?: boolean;
  onEdit?: () => void; onRemove?: () => void; onSendNow?: () => void; onSteer?: () => void;
}) {
  const first = item.text.trim().split('\n')[0];
  // One send button: Steer joins the running turn, Send now stops it first; the in-flight label follows the same choice
  const send = onSteer
    ? { run: onSteer, label: t('queue.steer'), busy: t('queue.steering'), icon: <Merge /> }
    : onSendNow && { run: onSendNow, label: t('queue.sendNow'), busy: t('queue.sending'), icon: <ArrowUp /> };
  return (
    <Row
      lead={<ListEnd className="size-icon" strokeWidth={1.5} />}
      title={item.sending ? send?.busy ?? t('queue.sending') : t('queue.title')}
      aria-busy={item.sending || undefined}
      className={cn(surfaceVariants({ tone: 'queue' }), 'px-pad py-1 text-1 text-fg-1')}
      trailing={(onEdit || onRemove) && <>
        {send && <IconButton title={item.sending ? send.busy : send.label} aria-label={send.label} disabled={disabled || sending} className="disabled:opacity-50" onClick={send.run}>{send.icon}</IconButton>}
        {onEdit && <IconButton title={t('queue.edit')} aria-label={t('queue.edit')} disabled={item.sending} className="disabled:opacity-50" onClick={onEdit}><Pencil /></IconButton>}
        {onRemove && <IconButton title={t('queue.remove')} aria-label={t('queue.remove')} disabled={item.sending} className="disabled:opacity-50" onClick={onRemove}><Trash2 /></IconButton>}
      </>}
    >
      {item.attachments.length > 0 && <AttachmentTiles attachments={item.attachments} blobUrl={blobUrl} />}
      {first && (
        <RowTarget className={onEdit && 'cursor-text'}>
          {onEdit
            ? <button type="button" disabled={item.sending} onClick={onEdit} className="max-w-full truncate rounded-md px-1.5 py-0.5 text-left align-middle transition-colors hover:bg-hover focus-visible:bg-hover">{first}</button>
            : first}
        </RowTarget>
      )}
    </Row>
  );
}

// The queued prompt goes out with whatever mode / model the session has when its turn comes, so the editor carries no option chips: text, kept attachments, new drafts
const NO_CONTROLS: SessionControls = { modes: [], options: [] };
const noop = () => undefined;

function QueuedEditor({ item, composer, blobUrl, onSave, onClose }: {
  item: QueuedPrompt; composer: ComposerProps; blobUrl?: (blob: string) => string;
  onSave: (text: string, retainedAttachments: number[], attachments: Draft[]) => void; onClose: () => void;
}) {
  const [retained, setRetained] = useState(() => item.attachments.map((_, i) => i));
  return (
    <div className="fade-in min-w-0">
      <Composer {...composer} running={false} disabled={false} controls={NO_CONTROLS} hidden={undefined} usage={undefined} turns={[]}
        onSetMode={noop} onSetConfig={noop}
        edit={{ text: item.text, hasAttachments: retained.length > 0, onCancel: onClose,
          attachments: <EditAttachments attachments={item.attachments} retained={retained} blobUrl={blobUrl} onRemove={i => setRetained(r => r.filter(n => n !== i))} /> }}
        onSend={(text, drafts) => { onSave(text, retained, drafts); onClose(); }}
      />
    </div>
  );
}
