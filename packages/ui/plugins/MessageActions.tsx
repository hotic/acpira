import { memo, useState } from 'react';
import type { PluginSlot } from '@acpira/shared/plugins';
import { pushShellToast } from '../desktop';
import { t } from '../i18n';
import { callPluginMethod, usePluginPieces } from '../pluginSlots';
import { cn } from '../ui/cn';

// The message.actions slot (docs/dev/plugins.md Interface): a plugin's buttons in the row under a finished reply, drawn
// natively (a frame per message would cost a document per turn), labelled with the piece's title. A click calls the
// plugin's runtime method with the session, the reply's transcript index and its Markdown; a failure is a toast, a
// result is the plugin's to show (a ui/post to its own pieces, a tool card)

// One click of a plugin's native button (message.actions, selection.actions): its runtime method, a failure as a toast
export async function runPluginAction(p: PluginSlot, params: { session: string; turn?: number; text: string }): Promise<void> {
  if (!p.method) return;
  try {
    await callPluginMethod(p.plugin, p.method, params);
  } catch (e) {
    pushShellToast({ type: 'toast', level: 'error', text: t('plugin.actionFailed', { name: p.title, error: e instanceof Error ? e.message : String(e) }) });
  }
}

export const PluginActions = memo(function PluginActions({ session, turn, text, slot = 'message.actions' }: { session: string; turn: number; text: string; slot?: 'message.actions' | 'prompt.actions' }) {
  const pieces = usePluginPieces(slot);
  const [busy, setBusy] = useState<string>();
  if (!pieces.length) return null;
  return <>
    {pieces.map(p => {
      const key = `${p.plugin}/${p.id}`;
      const run = async () => {
        if (!p.method || busy) return;
        setBusy(key);
        await runPluginAction(p, { session, turn, text });
        setBusy(undefined);
      };
      return (
        <button key={key} type="button" data-plugin-action={key} title={p.title} aria-busy={busy === key || undefined} disabled={busy === key} onClick={() => void run()}
          className={cn('inline-flex h-ctl-sm max-w-pop-sm items-center rounded-md px-1.5 text-3 text-fg-2 transition-colors hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1',
            busy === key && 'animate-pulse')}>
          <span className="truncate">{p.title}</span>
        </button>
      );
    })}
  </>;
});

// The prompt.actions slot: a plugin's buttons in a strip under a sent prompt, called with the prompt's transcript index
// and text. Acpira puts nothing of its own there, so without such a plugin no strip exists and a prompt sits one
// message gap above its reply. With one, the strip stands in for that gap (it cancels the gap on both sides and brings
// its own padding) and shows like a reply's action row: while the prompt above it (`peer/prompt` on HistoryMessage) or
// the strip itself is hovered / focused, and always on touch screens.
// As the exchange's last child there is no gap below to cancel, and a negative margin would push the strip past the
// exchange's paint containment, so `last:mb-0` keeps it inside
export function PromptActions({ session, turn, text }: { session?: string; turn: number; text: string }) {
  const pieces = usePluginPieces('prompt.actions');
  if (!session || !pieces.length) return null;
  return (
    <div data-plugin-slot="prompt.actions" className="-my-msg last:mb-0 box-content flex h-ctl-sm items-center justify-end gap-0.5 py-1 opacity-0 transition-opacity duration-(--code-copy-duration) ease-out peer-hover/prompt:opacity-100 peer-focus-within/prompt:opacity-100 hover:opacity-100 focus-within:opacity-100 [@media(hover:none)]:opacity-100 motion-reduce:transition-none">
      <PluginActions session={session} turn={turn} text={text} slot="prompt.actions" />
    </div>
  );
}
