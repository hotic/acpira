import { ArrowUp, Square } from 'lucide-react';
import { useAppearance } from '../appearance';
import { cn } from '../ui/cn';
import { t } from '../i18n';
import { MetalRing } from './MetalRing';

export interface SendButtonProps {
  running: boolean;
  filled: boolean;
  // metal mode hands the theme to the ring
  theme?: 'dark' | 'light';
  onClick?: () => void;
}

// Send / stop in one, a --ctl-sm flat button, one tier below the toolbar chips' neighbors. Dim when empty, lit when there's text or a run in progress.
// Three modes: accent lights up with the accent color; icon is inverted-neutral; metal adds a silver ring around the same solid surface
// (effects/MetalRing). The ring's wrapper owns the inverted fill and icon color; the button stays on screen whatever the ring does.
export function SendButton({ running, filled, theme = 'dark', onClick }: SendButtonProps) {
  const { send, motion } = useAppearance();
  const on = filled || running;
  const metal = send === 'metal' && on;
  const button = (
    <button
      type="button"
      aria-label={running ? t('composer.stop') : t('composer.send')}
      onClick={onClick}
      disabled={!on}
      data-on={on || undefined}
      data-metal={metal || undefined}
      className={cn(
        'send-btn inline-flex size-ctl-sm shrink-0 items-center justify-center rounded-md [&_svg]:size-icon disabled:cursor-default',
        send === 'accent' && on && '[--send-bg-on:var(--accent)] [--send-ink-on:var(--accent-fg)]',
      )}
    >
      {running ? <Square className="size-3! fill-current" strokeWidth={0} /> : <ArrowUp strokeWidth={2} />}
    </button>
  );
  if (!metal) return button;
  return (
    <MetalRing theme={theme} paused={motion === 'none'} className="shrink-0 rounded-md bg-btn-1 text-btn-1-fg">
      {button}
    </MetalRing>
  );
}
