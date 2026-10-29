import { memo, useState } from 'react';
import { pushShellToast } from '../desktop';
import { t } from '../i18n';
import { callPluginMethod, usePluginPieces } from '../pluginSlots';
import { cn } from '../ui/cn';

// The message.actions slot (docs/dev/plugins.md Interface): a plugin's buttons in the row under a finished reply, drawn
// natively (a frame per message would cost a document per turn), labelled with the piece's title. A click calls the
// plugin's runtime method with the session, the reply's transcript index and its Markdown; a failure is a toast, a
// result is the plugin's to show (a ui/post to its own pieces, a tool card)
export const PluginActions = memo(function PluginActions({ session, turn, text }: { session: string; turn: number; text: string }) {
  const pieces = usePluginPieces('message.actions');
  const [busy, setBusy] = useState<string>();
  if (!pieces.length) return null;
  return <>
    {pieces.map(p => {
      const key = `${p.plugin}/${p.id}`;
      const run = async () => {
        if (!p.method || busy) return;
        setBusy(key);
        try {
          await callPluginMethod(p.plugin, p.method, { session, turn, text });
        } catch (e) {
          pushShellToast({ type: 'toast', level: 'error', text: t('plugin.actionFailed', { name: p.title, error: e instanceof Error ? e.message : String(e) }) });
        } finally {
          setBusy(undefined);
        }
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
