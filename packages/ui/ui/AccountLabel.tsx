import { cn } from './cn';
import { t } from '../i18n';

// Vendors whose name leads a stored plan ("Claude Max", "ChatGPT Pro", Devin's whoami "Devin Pro"): the agent is
// already named by the menu or page the account sits in, so the tag keeps the plan alone. A product name that is the
// plan itself ("SuperGrok", Kimi's "Moderato") has no such leading word and stays whole
const VENDOR_LEAD = /^(?:claude|chatgpt|openai|codex|devin|windsurf|kimi|grok|gemini|google|factory|droid)\s+(?=\S)/i;

// The plan as its tag shows it
export const planName = (detail: string) => detail.replace(VENDOR_LEAD, '');

// Keep the account identity and plan on one line; only the identity may truncate. The plan is a small tag, the same
// chip the quota bars use for their countdowns
export function AccountLabel({ label, detail, className }: { label: string; detail?: string; className?: string }) {
  const plan = detail && planName(detail);
  return <span className={cn('flex min-w-0 items-center gap-2', className)} title={[label, detail].filter(Boolean).join(t('common.metaSep'))}>
    <span className="min-w-0 truncate">{label}</span>
    {plan && <span className="shrink-0 rounded-sm bg-active px-1 text-3 font-normal text-fg-2">{plan}</span>}
  </span>;
}
