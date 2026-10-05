import { AtSign } from 'lucide-react';
import { Chip } from '../ui/Button';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { t } from '../i18n';
import { AgentMark } from './AgentMark';
import type { MentionPersona } from './Mention';

// The composer's summon chip: the user's cross-harness subagents in a menu; picking one writes `@name ` into the draft, the
// same as choosing it from the @ list. The agent then calls it through ask_agent; the child runs in its own CLI, sees the
// repository but not this conversation, and its reply comes back here
export function SummonMenu({ personas, onPick, onOpenChange }: {
  personas: MentionPersona[];
  onPick: (persona: MentionPersona) => void;
  onOpenChange: (open: boolean) => void;
}) {
  return (
    <DropdownMenu.Root onOpenLifecycle={onOpenChange}>
      <DropdownMenu.Trigger render={<Chip narrow="icon" caret={false} className="shrink-0" icon={<AtSign strokeWidth={1.5} />}
        title={t('summon.hint')} aria-label={t('summon.entry')}>
        {t('summon.entry')}
      </Chip>} />
      <DropdownMenu.Portal><DropdownMenu.Positioner side="top" width="md"><DropdownMenu.Popup>
        {/* No caption above the list: what a summon does is the chip's tooltip. Two-line items get room above and below
            their text and a small gap between them, so the names do not stack into one block */}
        <DropdownMenu.Group className="scroll-thin flex max-h-pop flex-col gap-0.5 overflow-y-auto">
          {personas.map(p => (
            <DropdownMenu.Item key={p.id} className="py-1.5" onClick={() => onPick(p)}>
              {/* The CLI's mark spans both lines (lead size, not the one-line icon); the CLI's name outranks the model */}
              <AgentMark id={p.agent} name={p.name} className="size-lead shrink-0" />
              <OptionContent extra={(p.cli || p.model) && (
                <span className="flex min-w-0 items-center gap-gap text-3">
                  {p.cli && <span className="shrink-0 text-fg-1">{p.cli}</span>}
                  {p.model && <span className="min-w-0 truncate text-fg-3/70">{p.model}</span>}
                </span>
              )}>{p.name}</OptionContent>
            </DropdownMenu.Item>
          ))}
        </DropdownMenu.Group>
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
