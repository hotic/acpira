import { clsx, type ClassValue } from 'clsx';
import { extendTailwindMerge } from 'tailwind-merge';

// Numeric text tokens are font sizes, never colors. Keep text-2 with text-fg-2.
// `spacing` lists every --spacing-* key of the @theme block in styles/tokens.css (test/ui-classnames.test.ts checks it).
const merge = extendTailwindMerge({ extend: { theme: {
  text: ['1', '2', '3', '4', 'mono'],
  spacing: ['row', 'row-dense', 'ctl', 'ctl-sm', 'lead', 'icon', 'icon-ctl', 'tick', 'pad', 'pad-y', 'page', 'page-gutter', 'dock', 'dock-gutter', 'chat-column', 'gap', 'gap-half', 'code-output', 'code-line', 'diff-number', 'diff-sign', 'diff-gutter', 'diff-hunk', 'msg', 'msg-join', 'hdr', 'thumb', 'agent-image', 'widget-placeholder', 'widget-max', 'indent', 'hit', 'hit-end', 'term', 'install-log', 'command-x', 'command-y', 'command-after', 'pop', 'plan', 'question-body', 'plan-body', 'pop-sm', 'pop-md', 'pop-lg', 'pop-xl', 'project', 'unread-dot', 'session-time', 'subagent-pane', 'switch-track-h', 'switch-track-w', 'switch-thumb', 'switch-on', 'switch-off', 'switch-lg-h', 'switch-lg-w', 'switch-lg-pad', 'switch-lg-knob', 'switch-lg-on', 'subagent-mark'],
  shadow: ['card', 'pop'],
} } });
export const cn = (...inputs: ClassValue[]) => merge(clsx(inputs));

// Base UI also accepts state-dependent classes; preserve that native API.
export function cnState<State>(base: ClassValue, custom?: string | ((state: State) => string | undefined)) {
  return typeof custom === 'function' ? (state: State) => cn(base, custom(state)) : cn(base, custom);
}
