import { createContext, useContext, useLayoutEffect, useRef, useState, type MouseEvent } from 'react';
import { ArrowLeft, ArrowUpRight, ListTodo } from 'lucide-react';
import type { ConfigControl, PermissionBlock, PlanDocumentBlock, SessionControls } from '@shared/transcript';
import { groupModels, variantLabel } from '@shared/models';
import { composerControls, reasoningChip } from '@shared/composerControls';
import { Button, Chip, IconButton } from '../ui/Button';
import { Card } from '../ui/Card';
import { getLocale, t } from '../i18n';
import { Popover } from '../ui/Popover';
import { PanelFooter, PanelHeader, OptionContent } from '../ui/Panel';
import { RadioGroup } from '../ui/RadioGroup';
import { Row } from '../ui/Row';
import { cn } from '../ui/cn';
import { useScrollFade } from '../ui/useScrollFade';
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
  const canBuild = !busy && !!ctx.build && !!block.markdown;
  // The header carries the title, so a body that opens with the same `# Title` drops that line
  const { title, body } = planHeading(block);
  const status = permission && !started ? t('plan.pendingApproval') : t(`plan.status.${block.status}`);
  const fade = useScrollFade<HTMLDivElement>();
  return (
    <Card className="plan-document flex min-w-0 flex-col overflow-hidden" data-plan-document={block.id} data-plan-status={block.status}>
      {/* Header: what this is, where it stands, and the file behind it */}
      <div className="flex min-w-0 items-center gap-gap py-gap-half pr-gap-half pl-pad">
        <Row dense className="min-w-0 flex-1" lead={<ListTodo className="size-icon" strokeWidth={1.5} />}>
          <h3 className="m-0 min-w-0 truncate text-2 font-medium text-fg-strong" title={title}>{title}</h3>
        </Row>
        <span className={cn('shrink-0 text-3', permission && !started ? 'text-fg-1' : 'text-fg-3')}>{status}</span>
        <IconButton size="sm" className="shrink-0" title={t('plan.openFile')} aria-label={t('plan.openFile')}
          onClick={() => ctx.open?.(block.id)} disabled={!ctx.open}>
          <ArrowUpRight strokeWidth={1.5} />
        </IconButton>
      </div>
      {/* Body: indented to the header's label column; a long plan scrolls inside the card so the actions stay in reach */}
      {body && <div ref={fade} className="scroll-fade scroll-thin max-h-plan-body overflow-y-auto pt-gap-half pr-pad pb-pad pl-pad">
        <div className="pl-indent"><Prose block={{ type: 'text', markdown: body }} /></div>
      </div>}
      {!started && <div className="@container flex min-w-0 items-center justify-between gap-gap border-t border-line py-gap-half pr-gap-half pl-pad">
        {revise ? <Button variant="secondary" disabled={!ctx.ready} title={permissionOption(revise, getLocale(), true).label} onClick={() => choose(revise.id)}
          className="h-ctl-sm shrink-0">{t('plan.revise')}</Button> : <span />}
        <div className="ml-auto flex min-w-0 items-center gap-gap">
          {(model || extra.length > 0) && <Popover.Root open={menuOpen} onOpenChange={setMenuOpen}>
            <Popover.Trigger render={<Chip aria-label={t('plan.approvalsAria')} title={executor ?? t('plan.moreApprovals')}
              narrow="text" meta={meta} icon={family && <ModelMark family={family.name} brand={family.brand} />} data-plan-executor>
              {family?.name ?? executor ?? t('plan.approvals')}
            </Chip>} />
            <Popover.Portal><Popover.Positioner side="top" align="end" width={extra.length > 0 ? 'xl' : 'md'}><Popover.Popup>
              <BuildMenu model={model && { ...model, value: choice?.value ?? model.value }}
                hidden={model && ctx.hidden?.[model.id]} extra={extra} ready={ctx.ready} canBuild={canBuild}
                onSelect={value => model && setSelected({ configId: model.id, value })} onChoose={choose} close={() => setMenuOpen(false)} />
            </Popover.Popup></Popover.Positioner></Popover.Portal>
          </Popover.Root>}
          {/* The one primary action of the card: start with the chosen executor */}
          <Button variant="primary" className="h-ctl-sm shrink-0" disabled={!canBuild} data-plan-build
            onClick={() => ctx.build?.(block.id, choice, primary?.id)}>
            {primary ? permissionOption(primary, getLocale(), true).label : t('plan.build')}
          </Button>
        </div>
      </div>}
    </Card>
  );
}

const LEADING_H1 = /^#[ \t]+(.+?)[ \t]*#*[ \t]*(?:\r?\n|$)/;

// Header title and remaining body: the engine names a plan after its first `# heading` ("Plan" when it has none)
function planHeading(block: PlanDocumentBlock) {
  const markdown = block.markdown.trimStart();
  const h1 = LEADING_H1.exec(markdown);
  const body = h1 && h1[1] === block.title ? markdown.slice(h1[0].length).trim() : markdown.trim();
  return { title: block.title && block.title !== 'Plan' ? block.title : t('plan.title'), body };
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
