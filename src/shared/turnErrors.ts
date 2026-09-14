import type { TurnError } from './transcript';

// Provider retryable flags can describe transport policy even when the same
// model input cannot fit. Keep the original error intact for copying/logging.
export function isContextLengthError(error: TurnError | undefined): boolean {
  if (!error) return false;
  return ['context_length_exceeded', 'context_window_exceeded', 'prompt_too_long'].includes(error.kind ?? '')
    || /\bprompt (?:to the model )?(?:was|is) too long\b|\bcontext (?:length|window) (?:exceeded|limit exceeded)\b|\bmaximum context length\b/i.test(error.message);
}
