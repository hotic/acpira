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
        <DropdownMenu.Group className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
          {/* One short line above the list; a longer translation wraps at a normal leading instead of the tight text-3 one */}
          <DropdownMenu.GroupLabel className="px-2 pt-1 pb-1.5 text-3 leading-normal text-pretty text-fg-3">{t('summon.hint')}</DropdownMenu.GroupLabel>
          {personas.map(p => (
            <DropdownMenu.Item key={p.id} onClick={() => onPick(p)}>
              <OptionContent icon={<AgentMark id={p.agent} name={p.name} />} description={p.meta}>{p.name}</OptionContent>
            </DropdownMenu.Item>
          ))}
        </DropdownMenu.Group>
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
