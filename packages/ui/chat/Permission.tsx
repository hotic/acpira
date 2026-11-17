import { useRef, type RefObject } from 'react';
import { Terminal } from 'lucide-react';
import type { PermissionBlock } from '@acpira/shared/transcript';
import { Button } from '../ui/Button';
import { DropdownMenu } from '../ui/DropdownMenu';
import { Row } from '../ui/Row';
import { cn } from '../ui/cn';
import { getLocale, t } from '../i18n';
import { keyHint } from '../keybindings';
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
  // `over`: open above that element instead of above the trigger
  const menuWith = (trigger?: string, over?: RefObject<HTMLElement | null>) => (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger render={<Button caret className={trigger}>{t('permission.more')}</Button>} />
      <DropdownMenu.Portal><DropdownMenu.Positioner width="xl" side="top" anchor={over} collisionAvoidance={{ side: 'flip', align: 'shift' }}><DropdownMenu.Popup>
        <PermissionMenu options={more} onChoose={id => onChoose?.(id)} />
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
  const menu = more.length > 0 && menuWith(compact ? choice : undefined);
  const card = useRef<HTMLDivElement>(null);
  // The adapter asked for a deny-by-default card: reject is the emphasized button, allow stays plain
  const enter = keyHint('Enter');
  const rejectHint = reject ? [reject.detail, block.defaultToNo ? `${enter} / Esc` : 'Esc'].filter(Boolean).join(' · ') : undefined;
  const allowHint = allow ? [allow.detail, !block.defaultToNo ? enter : undefined].filter(Boolean).join(' · ') || undefined : undefined;
  const choices = flat
    ? options.map(o => <Button key={o.id} className={choice} title={o.detail} onClick={() => onChoose?.(o.id)}>{o.label}</Button>)
    : <>
      {reject && <Button className={choice} title={rejectHint} variant={block.defaultToNo ? 'primary' : undefined} onClick={() => onChoose?.(reject.id)}>{reject.label}</Button>}
      {allow && <Button className={choice} title={allowHint} variant={block.defaultToNo ? undefined : 'primary'} onClick={() => onChoose?.(allow.id)}>{allow.label}</Button>}
    </>;
  const details = <>
    {block.command && <pre className={cn('m-0 whitespace-pre-wrap font-mono text-mono [overflow-wrap:anywhere]', compact ? 'text-fg-2' : 'text-fg-1')}>{block.command}</pre>}
    {block.description && <p className="m-0 text-3 text-fg-2 [overflow-wrap:anywhere]">{block.description}</p>}
  </>;
  if (compact) {
    return (
      <div data-permission-card className="flex min-w-0 flex-col gap-gap-half rounded-md border border-line py-gap-half pr-gap-half pl-pad">
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
  // Bleed the frame into the message inset so its icon stays on the tool rows' keyline.
  // data-permission-card: the welcome tour's approvals step finds the card by it, whatever frames it.
  // A command takes two bands, as on the question card: the command itself behind its glyph, on the card's full width
  // however long it is, then a footer under a hairline with the menu at its start and the decision at its end. Buttons
  // beside a wrapped command squeezed it and hung mid-air. Any other request is one line of title with the same buttons
  // at its end; equal-choice requests list their answers under the title
  const foot = 'h-auto min-h-ctl-sm max-w-full whitespace-normal [overflow-wrap:anywhere]';
  const decide = <>
    {reject && <Button className={foot} title={rejectHint} variant={block.defaultToNo ? 'primary' : undefined} onClick={() => onChoose?.(reject.id)}>{reject.label}</Button>}
    {allow && <Button className={foot} title={allowHint} variant={block.defaultToNo ? undefined : 'primary'} onClick={() => onChoose?.(allow.id)}>{allow.label}</Button>}
  </>;
  const frame = '-mx-pad flex min-w-0 flex-col rounded-md bg-bg-1 shadow-[inset_0_0_0_1px_var(--conversation-line)]';
  const description = block.description && <p className={cn('m-0 text-3 text-fg-2 [overflow-wrap:anywhere]', block.command && 'pl-indent')}>{block.description}</p>;
  if (!block.command) {
    return (
      <div data-permission-card className={cn(frame, 'gap-gap-half py-2 pr-2 pl-pad')}>
        <div className="flex min-w-0 flex-wrap items-center gap-gap">
          <span className="min-w-0 flex-auto font-medium text-fg-1 [overflow-wrap:anywhere]">{title}</span>
          {!flat && <div className="ml-auto flex min-w-0 max-w-full flex-wrap items-center justify-end gap-gap">{more.length > 0 && menuWith('h-ctl-sm')}{decide}</div>}
        </div>
        {description}
        {flat && <div className="flex min-w-0 max-w-full flex-wrap items-center gap-gap">{choices}</div>}
      </div>
    );
  }
  return (
    <div ref={card} data-permission-card role="group" aria-label={title} className={frame}>
      <div className="flex min-w-0 flex-col gap-gap-half px-pad py-gap">
        <Row className="lead-top min-h-0 min-w-0 items-start text-fg-1 select-text" lead={<Terminal className="size-icon" strokeWidth={1.5} />}>
          <pre className="m-0 min-w-0 whitespace-pre-wrap font-mono text-mono [overflow-wrap:anywhere]">{block.command}</pre>
        </Row>
        {description}
        {flat && <div className="flex min-w-0 max-w-full flex-wrap items-center gap-gap pt-gap-half">{choices}</div>}
      </div>
      {!flat && (
        <div className="flex min-w-0 flex-wrap items-center justify-end gap-gap border-t border-line p-2">
          {/* The menu opens above the card, so the command it widens stays readable under it */}
          {more.length > 0 && <span className="mr-auto flex min-w-0">{menuWith('h-ctl-sm', card)}</span>}
          {decide}
        </div>
      )}
    </div>
  );
}

type DisplayOption = ReturnType<typeof permissionOption>;

function PermissionMenu({ options, onChoose }: { options: DisplayOption[]; onChoose: (id: string) => void }) {
  const commonDetail = options.find(o => o.detail)?.detail;
  const sharedDetail = commonDetail && options.filter(o => !o.bypass).every(o => o.detail === commonDetail) ? commonDetail : undefined;
  const ordered = [...options.filter(o => !o.bypass), ...options.filter(o => o.bypass)];
  return (
    <div className="scroll-none max-h-pop overflow-y-auto">
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
