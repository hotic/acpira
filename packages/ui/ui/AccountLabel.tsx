import { cn } from './cn';
import { t } from '../i18n';

// Vendors whose name leads a stored plan ("Claude Max", "ChatGPT Pro", Devin's whoami "Devin Pro"): the agent is
// already named by the menu or page the account sits in, so the tag keeps the plan alone. A product name that is the
// plan itself ("SuperGrok", Kimi's "Moderato") has no such leading word and stays whole
const VENDOR_LEAD = /^(?:claude|chatgpt|openai|codex|devin|windsurf|kimi|grok|gemini|google|factory|droid)\s+(?=\S)/i;

// The plan as its tag shows it
export const planName = (detail: string) => detail.replace(VENDOR_LEAD, '');

// What is wrong with a login, in the plan tag's place: `warn` needs the user (sign in again, unlock), otherwise it passes
// by itself. `title` is the full sentence
export interface AccountState { text: string; warn?: boolean; title?: string }

// Keep the account identity and plan on one line; only the identity may truncate. The plan is a small tag, the only
// chip of an account row. A login with something wrong shows that in the tag instead of its plan; a long state
// is one short word in every language (the hint has the sentence), so the identity keeps the row: the tag gives way
// only past half of it
export function AccountLabel({ label, detail, state, className }: { label: string; detail?: string; state?: AccountState; className?: string }) {
  const plan = detail && planName(detail);
  return <span className={cn('flex min-w-0 items-center gap-2', className)} title={[label, detail].filter(Boolean).join(t('common.metaSep'))}>
    <span className="min-w-0 truncate">{label}</span>
    {state
      ? <span title={state.title ?? state.text} className={cn('max-w-1/2 shrink-0 truncate rounded-sm px-1 text-3 font-normal', state.warn ? 'bg-warn/15 text-warn' : 'bg-active text-fg-3')}>{state.text}</span>
      : plan && <span className="shrink-0 rounded-sm bg-active px-1 text-3 font-normal text-fg-2">{plan}</span>}
  </span>;
}
