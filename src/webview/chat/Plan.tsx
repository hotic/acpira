import { memo, useState, type ReactNode } from 'react';
import { Check, ChevronDown, ChevronsDown, ChevronsUp, ListTodo } from 'lucide-react';
import type { PlanBlock, PlanEntry, PlanStatus } from '@shared/transcript';
import { useAppearance } from '../appearance';
import { t } from '../i18n';
import { Card } from '../ui/Card';
import { Collapsible } from '../ui/Collapsible';
import { Disclosure } from '../ui/Disclosure';
import { Row, RowLabel } from '../ui/Row';
import { cn } from '../ui/cn';
import { useScrollFade } from '../ui/useScrollFade';

// Dock entries use balanced text padding; the header retains its full hit area.
const dockEntryClass = 'min-h-[calc(var(--text-2-lh)+var(--plan-entry-pad-y)*2)] py-(--plan-entry-pad-y)';

// Keep the current entry mounted while the surrounding rows expand. Sharing
// the same row avoids a disappearing summary and a shrinking first frame.
export function PlanCard({ entries, live }: { entries: PlanEntry[]; live?: boolean }) {
  const { toolLine } = useAppearance();
  const [open, setOpen] = useState(false);
  const fade = useScrollFade<HTMLDivElement>();
  const done = entries.filter(e => e.status === 'completed').length;
  const current = entries.find(e => e.status === 'in_progress') ?? entries.find(e => e.status === 'pending');
  const lead = toolLine === 'text' ? undefined : <ListTodo className="size-icon" strokeWidth={1.5} />;
  return (
    <Card className="flex min-w-0 flex-col gap-(--plan-section-gap) px-pad py-(--plan-pad-y)" data-open={open || undefined}>
      <Row
        as="button" interactive dense aria-expanded={open} onClick={() => setOpen(o => !o)}
        lead={lead}
        trailing={<>
          <span>{done}/{entries.length}</span>
          <PlanCaret open={open} />
        </>}
      >
        <RowLabel className={cn(live && current && 'shimmer')}>{t('plan.todoTitle')}</RowLabel>
      </Row>
      <div ref={fade} className="scroll-fade scroll-thin scroll-stable max-h-plan overflow-y-auto [--scroll-fade-size:var(--gap)] [overflow-anchor:none]">
        <Collapsible.Root open={!open && !current}><Collapsible.Panel>
          <Row className={cn('items-start lead-top', dockEntryClass)} dense lead={toolLine === 'text' ? undefined : <PlanDot status="completed" />}>
            <span className="text-fg-2">{t('plan.completed', { n: entries.length })}</span>
          </Row>
        </Collapsible.Panel></Collapsible.Root>
        <PlanEntries entries={entries} expanded={open} current={current} showStatus={toolLine !== 'text'} entryClass={dockEntryClass} />
      </div>
    </Card>
  );
}

// Historical plans are a single disclosure row. Expanded entries keep their
// natural height and participate in the conversation's normal scroll flow.
export const Plan = memo(function Plan({ block }: { block: PlanBlock }) {
  return <PlanDetails entries={block.entries} label={<RowLabel>{t('plan.title')}</RowLabel>} />;
});

// Standard plans and todo-tool results share the same indented body and single
// title-aligned rail. Entry status icons are not anchors for this outer rail.
export function PlanDetails({ entries, label, trailing }: { entries: PlanEntry[]; label: ReactNode; trailing?: ReactNode }) {
  const { toolLine } = useAppearance();
  const [open, setOpen] = useState(false);
  const done = entries.filter(entry => entry.status === 'completed').length;
  return (
    <Disclosure className="action-details" tone="action"
      open={open} onToggle={setOpen}
      lead={toolLine === 'text' ? undefined : <ListTodo className="size-icon" strokeWidth={1.5} />}
      trailing={trailing}
      body={<PlanEntries entries={entries} />}
    >
      {label}
      <span className="text-3 text-fg-3 tabular-nums">{done}/{entries.length}</span>
      <PlanCaret open={open} />
    </Disclosure>
  );
}

function PlanCaret({ open }: { open: boolean }) {
  return <ChevronDown className={cn('size-icon shrink-0 self-center transition-transform', open && 'rotate-180')} strokeWidth={1.5} />;
}

// The entry list shared by the transcript block and the PlanBar: status dot in the lead slot, the title dimming once done, a faint priority glyph on the right for high / low
export function PlanEntries({ entries, expanded, current, showStatus = true, entryClass }: {
  entries: PlanEntry[];
  expanded?: boolean;
  current?: PlanEntry;
  showStatus?: boolean;
  entryClass?: string;
}) {
  return (
    <ol className="flex flex-col">
      {entries.map((entry, index) => {
        const key = `${index}:${entry.title}`;
        const row = <PlanEntryRow key={key} entry={entry} showStatus={showStatus} className={entryClass} />;
        // History keeps its natural layout. Only the live dock folds individual rows.
        return expanded === undefined ? row : (
          <Collapsible.Root key={key} open={expanded || entry === current}><Collapsible.Panel>{row}</Collapsible.Panel></Collapsible.Root>
        );
      })}
    </ol>
  );
}

function PlanEntryRow({ entry, showStatus, className }: { entry: PlanEntry; showStatus: boolean; className?: string }) {
  return (
    <Row className={cn('items-start lead-top', className)} dense lead={showStatus ? <PlanDot status={entry.status} /> : undefined} trailing={priorityGlyph(entry.priority)}>
      <span className={cn('min-w-0 break-words', entry.status === 'completed' ? 'text-fg-2' : 'text-fg-1', entry.status === 'in_progress' && 'text-fg-strong')}>
        {entry.title}
      </span>
    </Row>
  );
}

// Medium is the default and gets no mark
function priorityGlyph(priority: PlanEntry['priority']): ReactNode | undefined {
  if (priority === 'high') return <ChevronsUp className="size-3" strokeWidth={1.75} aria-label={t('plan.priority.high')} />;
  if (priority === 'low') return <ChevronsDown className="size-3" strokeWidth={1.75} aria-label={t('plan.priority.low')} />;
  return undefined;
}

export function PlanDot({ status }: { status: PlanStatus }) {
  if (status === 'completed') return <span className="flex size-3 items-center justify-center rounded-full bg-fg-3 text-bg-0"><Check className="size-2" strokeWidth={3} /></span>;
  if (status === 'in_progress') return <span className="flex size-3 items-center justify-center rounded-full border-[1.5px] border-accent"><span className="size-1.5 rounded-full bg-accent" /></span>;
  return <span className="size-3 rounded-full border-[1.5px] border-fg-3" />;
}
