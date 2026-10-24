import type { Draft } from './transcript';
import { MAX_TEXT_BYTES } from './attachments';

export const PASTE_TEXT_CHARS = 2000;
export const PASTE_TEXT_LINES = 20;

// Intercept large pastes before the textarea lays them out. Keep the payload verbatim.
export function collectPastedText(text: string, name: string): { draft?: Draft; tooBig?: boolean } {
  if (text.length < PASTE_TEXT_CHARS && text.split(/\r\n|\r|\n/, PASTE_TEXT_LINES).length < PASTE_TEXT_LINES) return {};
  if (new TextEncoder().encode(text).byteLength > MAX_TEXT_BYTES) return { tooBig: true };
  return { draft: { kind: 'text', name, text } };
}
