import { useEffect, useState } from 'react';
import type { ExternalSessionInfo } from '@shared/transcript';
import { Button } from '../ui/Button';
import { Card } from '../ui/Card';
import { t } from '../i18n';

// This is a receiver, not a disabled ACP connection. No login, model, send or stop affordance.
export function ExternalSessionNotice({ info }: { info: ExternalSessionInfo }) {
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState(false);
  useEffect(() => { setCopied(false); setCopyError(false); }, [info.sourceKey]);
  const copy = async () => {
    try { await navigator.clipboard.writeText(info.connectionPrompt!); setCopied(true); setCopyError(false); }
    catch { setCopyError(true); }
  };
  return <div className="px-page py-gap" data-chatgpt-mirror={info.state}>
    <Card className="flex flex-col gap-gap p-pad">
      <div className="flex flex-wrap items-center justify-between gap-gap">
        <span className="text-2 font-medium text-fg-strong">ChatGPT {t('chatgpt.mirror')}</span>
        <span role="status" aria-live="polite" className="text-3 text-fg-2">{t(`chatgpt.${info.state}`)}</span>
      </div>
      <p className="m-0 text-3 text-fg-2">{t('chatgpt.boundary')}</p>
      {info.connectionPrompt && <div className="flex flex-wrap items-center justify-between gap-gap">
        <span className="text-3 text-fg-3" title={info.lastEventAt}>{info.state === 'unbound' ? t('chatgpt.noEvent') : `${t('chatgpt.lastEvent')} ${new Date(info.lastEventAt).toLocaleTimeString()}`}</span>
        <Button onClick={() => void copy()}>{t(copied ? 'chatgpt.copyDone' : 'chatgpt.copy')}</Button>
      </div>}
      {copyError && <details open className="text-3 text-fg-2">
        <summary>{t('chatgpt.copyError')}</summary>
        <pre className="scroll-thin max-h-pop overflow-auto whitespace-pre-wrap select-text">{info.connectionPrompt}</pre>
      </details>}
    </Card>
  </div>;
}
