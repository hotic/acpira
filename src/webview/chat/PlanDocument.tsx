import { createContext, useContext, useLayoutEffect, useRef, useState, type MouseEvent } from 'react';
import { ArrowLeft, ArrowUpRight } from 'lucide-react';
import type { ConfigControl, PermissionBlock, PlanDocumentBlock, SessionControls } from '@shared/transcript';
import { groupModels, variantLabel } from '@shared/models';
import { composerControls, reasoningChip } from '@shared/composerControls';
import { Button, Chip, IconButton } from '../ui/Button';
import { Card } from '../ui/Card';
import { getLocale, t } from '../i18n';
import { Popover } from '../ui/Popover';
import { PanelFooter, PanelHeader, OptionContent } from '../ui/Panel';
import { RadioGroup } from '../ui/RadioGroup';
import { SendButton } from '../effects/SendButton';
import { ModelOptions } from './ModelPicker';
import { ModelMark } from './ModelMark';
import { Prose } from './Prose';
import { permissionOption } from './permissionOptions';

type ExecutionModel = { configId: string; value: string };
export const PlanDocumentContext = createContext<{
  controls: SessionControls;
  hidden?: Record<string, string[]>;
  running: boolean;
  ready: boolean;
  theme?: 'dark' | 'light';
  permissions?: PermissionBlock[];
  build?: (planId: string, model?: ExecutionModel, optionId?: string) => void;
  open?: (planId: string) => void;
}>({ controls: { modes: [], options: [] }, running: false, ready: false });

// Saved plans stay outside the process fold. Model selection is local until Build,
// so choosing an executor never changes the model generating the current plan.
export function PlanDocument({ block, permission: suppliedPermission, onChoose }: {
  block: PlanDocumentBlock;
  permission?: PermissionBlock;
  onChoose: (blockId: string, optionId: string) => void;
}) {
  const ctx = useContext(PlanDocumentContext);
  const permission = suppliedPermission ?? ctx.permissions?.find(p => p.planId === block.id);
  const [menuOpen, setMenuOpen] = useState(false);
  const [selected, setSelected] = useState<ExecutionModel>();
  const model = ctx.controls.options.find(c => c.category === 'model');
  const choice = model && selected?.configId === model.id && model.options.some(o => o.id === selected.value)
    ? selected : model?.value ? { configId: model.id, value: model.value } : undefined;
  const family = model && groupModels(model.options).find(f => f.variants.some(v => v.id === choice?.value));
  const variant = family?.variants.find(v => v.id === choice?.value);
  const params = family && variant && (family.efforts.length > 1 || variant.effort || variant.fast || variant.long)
    ? variantLabel(variant, family, { standard: t('composer.standard') }) : undefined;
  const levels = composerControls(ctx.controls.options).reasoning.map(reasoningChip);
  const meta = [params, ...levels].filter(Boolean).join(' ') || undefined;
  // Match the composer chip; source identity remains in the expanded model list.
  const executor = family && variant
    ? [family.name, meta].filter(Boolean).join(' ')
    : model?.options.find(o => o.id === choice?.value)?.name;
  const busy = !ctx.ready || (ctx.running && !permission) || block.status === 'draft' || block.status === 'executing';
  const primary = permission?.options.find(o => o.kind === 'allow_once');
  // Kimi's reject-and-exit is distinct from revising in Plan mode; keep it in the
  // additional approvals menu instead of relabeling it as a revision.
  const revise = permission?.options.find(o => o.kind === 'reject_once' && !/exit/i.test(o.id) && !/退出/.test(o.label));
  const extra = permission?.options.filter(o => o.id !== primary?.id && o.id !== revise?.id) ?? [];
  const started = block.status === 'approved' || block.status === 'executing';
  const choose = (optionId: string) => {
    const option = permission?.options.find(o => o.id === optionId);
    if (option?.kind.startsWith('allow')) ctx.build?.(block.id, choice, optionId);
    else if (permission) onChoose(permission.id, optionId);
  };
  return (
    <Card className="plan-document flex min-w-0 flex-col px-pad py-gap" data-plan-document={block.id}>
      <div className="relative min-w-0">
        <Prose block={{ type: 'text', markdown: block.markdown }} />
        <IconButton size="sm" className="absolute right-0 top-0" title={t('plan.openFile')} aria-label={t('plan.openFile')}
          onClick={() => ctx.open?.(block.id)} disabled={!ctx.open}>
          <ArrowUpRight strokeWidth={1.5} />
        </IconButton>
      </div>
      {!started && <div className="@container flex min-w-0 items-center justify-between gap-gap pt-gap">
        {revise ? <Button variant="secondary" disabled={!ctx.ready} title={permissionOption(revise, getLocale(), true).label} onClick={() => choose(revise.id)}
          className="shrink-0">{t('plan.revise')}</Button> : <span />}
        <div className="ml-auto flex min-w-0 items-center gap-gap">
          {(model || extra.length > 0) && <Popover.Root open={menuOpen} onOpenChange={setMenuOpen}>
            <Popover.Trigger render={<Chip aria-label={t('plan.approvalsAria')} title={executor ?? t('plan.moreApprovals')}
              narrow="text" meta={meta} icon={family && <ModelMark family={family.name} brand={family.brand} />} data-plan-executor>
              {family?.name ?? executor ?? t('plan.approvals')}
            </Chip>} />
            <Popover.Portal><Popover.Positioner side="top" align="end" width="md"><Popover.Popup>
              <BuildMenu model={model && { ...model, value: choice?.value ?? model.value }}
                hidden={model && ctx.hidden?.[model.id]} extra={extra} ready={ctx.ready} canBuild={!busy && !!ctx.build && !!block.markdown}
                onSelect={value => model && setSelected({ configId: model.id, value })} onChoose={choose} close={() => setMenuOpen(false)} />
            </Popover.Popup></Popover.Positioner></Popover.Portal>
          </Popover.Root>}
          <SendButton running={false} filled={!busy && !!ctx.build && !!block.markdown} theme={ctx.theme}
            onClick={() => ctx.build?.(block.id, choice, primary?.id)} />
        </div>
      </div>}
    </Card>
  );
}

// Additional permission choices retain their original ACP IDs and localize known labels. They
// live on a separate menu page so the model list remains a homogeneous list.
function BuildMenu({ model, hidden, extra, ready, canBuild, onSelect, onChoose, close }: {
  model?: ConfigControl;
  hidden?: string[];
  extra: PermissionBlock['options'];
  ready: boolean;
  canBuild: boolean;
  onSelect: (value: string) => void;
  onChoose: (optionId: string) => void;
  close: () => void;
}) {
  const [approvals, setApprovals] = useState(!model);
  const approvalList = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (approvals) approvalList.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus({ preventScroll: true });
  }, [approvals]);
  const showPage = (event: MouseEvent<HTMLButtonElement>, next: boolean) => {
    // Keep focus inside the popup before removing the page's focused button.
    // Base UI otherwise restores focus to the popup after the new row receives it.
    event.currentTarget.closest<HTMLElement>('[role="dialog"]')?.focus({ preventScroll: true });
    setApprovals(next);
  };
  if (approvals) return <>
    <PanelHeader lead={model ? { label: t('plan.backToExecutor'), icon: <ArrowLeft />, onClick: event => showPage(event, false) } : undefined}>{t('plan.moreApprovals')}</PanelHeader>
    <RadioGroup.Root ref={approvalList} value={null} aria-label={t('plan.moreApprovals')} className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
      {extra.map(o => <RadioGroup.Item key={o.id} value={o.id} disabled={o.kind.startsWith('allow') ? !canBuild : !ready}
        onClick={() => { onChoose(o.id); close(); }}><OptionContent>{permissionOption(o, getLocale(), true).label}</OptionContent></RadioGroup.Item>)}
    </RadioGroup.Root>
  </>;
  return <>
    <PanelHeader>{t('plan.executor')}</PanelHeader>
    {model && <ModelOptions control={model} hidden={hidden} onSelect={onSelect} close={close} />}
    {extra.length > 0 && <PanelFooter onClick={event => showPage(event, true)}>{t('plan.moreApprovals')}</PanelFooter>}
  </>;
}
