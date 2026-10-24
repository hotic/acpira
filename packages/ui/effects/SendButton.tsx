import { ArrowUp, Square } from 'lucide-react';
import { MetalFx } from 'metal-fx';
import { useAppearance } from '../appearance';
import { cn } from '../ui/cn';
import { t } from '../i18n';
import { METAL_PRESET, METAL_VARIANT } from './presets';
import { hasWebGL } from './webgl';

export interface SendButtonProps {
  running: boolean;
  filled: boolean;
  // metal mode hands the theme to metal-fx (it follows the OS prefers-color-scheme by default, not the shell)
  theme?: 'dark' | 'light';
  onClick?: () => void;
}

// metal-fx throws directly inside an effect when there's no WebGL, tearing down the whole React tree; without it the button keeps the inverted-neutral look
// Send / stop in one, a --ctl-sm flat button, one tier below the toolbar chips' neighbors. Dim when empty, lit when there's text or a run in progress.
// Three modes: accent lights up with the accent color; icon is inverted-neutral; metal adds a silver ring around the same solid surface.
// MetalFx clears the child's background, so the wrapper owns the inverted fill and icon color.
// MetalFx also keeps its whole wrapper (button included) at opacity 0 / visibility hidden until the shared shader has painted a first frame.
// A lost or stalled WebGL context in a long-lived webview therefore hid the send / stop button entirely; the visibility overrides below
// keep the solid button on screen regardless and let the ring fade in whenever the shader catches up.
export function SendButton({ running, filled, theme = 'dark', onClick }: SendButtonProps) {
  const { send, motion } = useAppearance();
  const on = filled || running;
  const metal = send === 'metal' && on && hasWebGL();
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
    <MetalFx variant={METAL_VARIANT} preset={METAL_PRESET} theme={theme} paused={motion === 'none'} className="shrink-0 rounded-md bg-btn-1! text-btn-1-fg! visible! opacity-100!">
      {button}
    </MetalFx>
  );
}
