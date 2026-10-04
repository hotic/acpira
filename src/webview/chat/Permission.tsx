import { ChevronDown, Terminal } from 'lucide-react';
import type { PermissionBlock } from '@shared/transcript';
import { Button } from '../ui/Button';
import { DropdownMenu } from '../ui/DropdownMenu';
import { Row } from '../ui/Row';
import { cn } from '../ui/cn';
import { getLocale, t } from '../i18n';
import { ambiguousChoices, permissionOption, permissionTitle, quickChoices } from './permissionOptions';

// Keep common decisions visible; the menu preserves every remaining wire option.
// `compact`: a summoned child's approval under its own row (the row already names who asks), one titled line with
// the same buttons, the command and description kept readable below it
export function Permission({ block, onChoose, compact }: { block: PermissionBlock; onChoose?: (optionId: string) => void; compact?: boolean }) {
  const options = block.options.map(o => permissionOption(o, getLocale(), !!block.planId));
  const flat = ambiguousChoices(options);
  // Positional quick buttons: first allow_once / reject_once in wire order, whatever the label says
  const { allow, reject } = flat ? {} : quickChoices(options);
  const more = flat ? [] : options.filter(o => o.id !== allow?.id && o.id !== reject?.id);
  const choice = compact ? 'h-ctl-sm max-w-full px-2' : 'h-auto min-h-ctl max-w-full whitespace-normal py-gap-half [overflow-wrap:anywhere]';
  const title = block.command ? t('permission.run') : permissionTitle(block.title, getLocale());
  const menu = more.length > 0 && (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger render={<Button className={compact ? choice : undefined}>{t('permission.more')}<ChevronDown className="size-icon shrink-0" strokeWidth={1.5} /></Button>} />
      <DropdownMenu.Portal><DropdownMenu.Positioner width="md" side="top" collisionAvoidance={{ side: 'flip', align: 'shift' }}><DropdownMenu.Popup>
        <PermissionMenu options={more} onChoose={id => onChoose?.(id)} />
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
  // The adapter asked for a deny-by-default card: reject is the emphasized button, allow stays plain
  const choices = flat
    ? options.map(o => <Button key={o.id} className={choice} title={o.detail} onClick={() => onChoose?.(o.id)}>{o.label}</Button>)
    : <>
      {reject && <Button className={choice} variant={block.defaultToNo ? 'primary' : undefined} onClick={() => onChoose?.(reject.id)}>{reject.label}</Button>}
      {allow && <Button className={choice} variant={block.defaultToNo ? undefined : 'primary'} onClick={() => onChoose?.(allow.id)}>{allow.label}</Button>}
    </>;
  const details = <>
    {block.command && <pre className={cn('m-0 whitespace-pre-wrap font-mono text-mono [overflow-wrap:anywhere]', compact ? 'text-fg-2' : 'text-fg-1')}>{block.command}</pre>}
    {block.description && <p className="m-0 text-3 text-fg-2 [overflow-wrap:anywhere]">{block.description}</p>}
  </>;
  if (compact) {
    return (
      <div className="flex min-w-0 flex-col gap-gap-half rounded-md border border-line py-gap-half pr-gap-half pl-pad">
        <div className="flex min-w-0 flex-wrap items-center gap-gap">
          <Row dense className="min-w-0 flex-1 text-fg-1" lead={block.command ? <Terminal className="size-icon" strokeWidth={1.5} /> : undefined}>
            <span className="min-w-0 truncate" title={title}>{title}</span>
          </Row>
          <div className="ml-auto flex min-w-0 max-w-full flex-wrap items-center justify-end gap-gap">{menu}{choices}</div>
        </div>
        {details}
      </div>
    );
  }
  return (
    <div className="flex min-w-0 flex-col gap-gap rounded-lg border border-conversation-line bg-bg-1 p-pad">
      <Row dense lead={block.command ? <Terminal className="size-icon" strokeWidth={1.5} /> : undefined}>
        <span className="font-medium">{title}</span>
      </Row>
      {details}
      <div className="flex flex-wrap items-center justify-between gap-gap">
        {menu}
        {flat ? (
          <div className="flex min-w-0 max-w-full flex-wrap items-center gap-gap">{choices}</div>
        ) : (
          <div className="ml-auto flex min-w-0 max-w-full flex-wrap items-center justify-end gap-gap">{choices}</div>
        )}
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
