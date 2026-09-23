import { ChevronDown, Terminal } from 'lucide-react';
import type { PermissionBlock } from '@shared/transcript';
import { Button } from '../ui/Button';
import { DropdownMenu } from '../ui/DropdownMenu';
import { Row } from '../ui/Row';
import { getLocale, t } from '../i18n';
import { permissionOption, quickChoices } from './permissionOptions';

// Keep common decisions visible; the menu preserves every remaining wire option.
export function Permission({ block, onChoose }: { block: PermissionBlock; onChoose?: (optionId: string) => void }) {
  const options = block.options.map(o => permissionOption(o, getLocale()));
  // Positional quick buttons: first allow_once / reject_once in wire order, whatever the label says
  const { allow, reject } = quickChoices(options);
  const more = options.filter(o => o.id !== allow?.id && o.id !== reject?.id);
  // The adapter asked for a deny-by-default card: reject is the emphasized button, allow stays plain
  return (
    <div className="flex min-w-0 flex-col gap-gap rounded-lg border border-conversation-line bg-bg-1 p-pad">
      <Row dense lead={block.command ? <Terminal className="size-icon" strokeWidth={1.5} /> : undefined}>
        <span className="font-medium">{block.command ? t('permission.run') : block.title}</span>
      </Row>
      {block.command && <pre className="m-0 whitespace-pre-wrap font-mono text-mono text-fg-1 [overflow-wrap:anywhere]">{block.command}</pre>}
      {block.description && <p className="m-0 text-3 text-fg-2 [overflow-wrap:anywhere]">{block.description}</p>}
      <div className="flex flex-wrap items-center justify-between gap-gap">
        {more.length > 0 && (
          <DropdownMenu.Root>
            <DropdownMenu.Trigger render={<Button>{t('permission.more')}<ChevronDown className="size-icon shrink-0" strokeWidth={1.5} /></Button>} />
            <DropdownMenu.Portal><DropdownMenu.Positioner width="md" side="top" collisionAvoidance={{ side: 'flip', align: 'shift' }}><DropdownMenu.Popup>
              <PermissionMenu options={more} onChoose={id => onChoose?.(id)} />
            </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
          </DropdownMenu.Root>
        )}
        <div className="ml-auto flex items-center gap-gap">
          {reject && <Button variant={block.defaultToNo ? 'primary' : undefined} onClick={() => onChoose?.(reject.id)}>{reject.label}</Button>}
          {allow && <Button variant={block.defaultToNo ? undefined : 'primary'} onClick={() => onChoose?.(allow.id)}>{allow.label}</Button>}
        </div>
      </div>
    </div>
  );
}

type DisplayOption = ReturnType<typeof permissionOption>;

function PermissionMenu({ options, onChoose }: { options: DisplayOption[]; onChoose: (id: string) => void }) {
  const commonDetail = options.find(o => o.detail)?.detail;
  const sharedDetail = commonDetail && options.filter(o => !o.bypass).every(o => o.detail === commonDetail) ? commonDetail : undefined;
  const ordered = [...options.filter(o => !o.bypass), ...options.filter(o => o.bypass)];
  return (
    <div className="scroll-thin max-h-pop overflow-y-auto">
      {ordered.map((option, index) => (
        <div key={option.id} className={option.bypass && index > 0 && !ordered[index - 1]?.bypass ? 'border-t border-line' : undefined}>
          {/* Menu rows stay inside the panel; conversation rows extend their hit area. */}
          <DropdownMenu.Item render={<Row as="button" />} onClick={() => onChoose(option.id)} className="w-full cursor-pointer rounded-md px-gap text-fg-2">
            <span className="min-w-0 whitespace-normal text-fg-1 [overflow-wrap:anywhere]">
              <span className="block">{option.label}</span>
              {!sharedDetail && option.detail && <span className="block text-3 text-fg-3">{option.detail}</span>}
            </span>
          </DropdownMenu.Item>
        </div>
      ))}
    </div>
  );
}
