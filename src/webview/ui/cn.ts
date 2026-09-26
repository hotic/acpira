import { clsx, type ClassValue } from 'clsx';
import { extendTailwindMerge } from 'tailwind-merge';

// Numeric text tokens are font sizes, never colors. Keep text-2 with text-fg-2.
const merge = extendTailwindMerge({ extend: { theme: {
  text: ['1', '2', '3', '4', 'mono'],
  spacing: ['row', 'row-dense', 'ctl', 'ctl-sm', 'lead', 'icon', 'icon-ctl', 'pad', 'pad-y', 'page', 'gap', 'msg', 'hdr', 'thumb', 'indent', 'hit', 'term', 'pop', 'plan', 'question-body', 'pop-sm', 'pop-md', 'pop-lg', 'pop-xl', 'switch-track-h', 'switch-track-w', 'switch-thumb', 'switch-on', 'switch-off', 'switch-lg-h', 'switch-lg-w', 'switch-lg-pad', 'switch-lg-knob', 'switch-lg-on', 'subagent-mark'],
  shadow: ['card', 'pop'],
} } });
export const cn = (...inputs: ClassValue[]) => merge(clsx(inputs));

// Base UI also accepts state-dependent classes; preserve that native API.
export function cnState<State>(base: ClassValue, custom?: string | ((state: State) => string | undefined)) {
  return typeof custom === 'function' ? (state: State) => cn(base, custom(state)) : cn(base, custom);
}
