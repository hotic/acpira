import { useMemo, useState } from 'react';
import type { ConfigControl } from '@shared/transcript';
import { findFusionVariant, findVariant, fusionLabel, groupModels, optionBrand, variantLabel, visibleOptions, type ModelFamily, type ModelVariant } from '@shared/models';
import { presentReasoning, reasoningChip, reasoningVisible } from '@shared/composerControls';
import { t } from '../i18n';
import { cn } from '../ui/cn';
import { Chip } from '../ui/Button';
import { RadioPills, SelectRow, SwitchRow } from '../ui/Field';
import { Popover } from '../ui/Popover';
import { DropdownMenu } from '../ui/DropdownMenu';
import { Command } from '../ui/Command';
import { OptionContent } from '../ui/Panel';
import { ModelMark } from './ModelMark';

// Only offer search once the option count passes this threshold; short lists are scannable at a glance
const SEARCH_FROM = 12;

interface OptionMenuProps {
  control: ConfigControl;
  onSelect: (value: string) => void;
  onOpenChange: (open: boolean) => void;
  // Right-side controls (the model slot) align their panel to the chip's right edge so it doesn't overflow the composer
  end?: boolean;
}

// Non-model options stay flat; reasoning must never be parsed as model families.
export function OptionControl({ control, hidden, ...rest }: OptionMenuProps & { hidden?: string[] }) {
  const shown = useMemo(() => ({ ...control, options: visibleOptions(control.options, hidden, control.value) }), [control, hidden]);
  return <OptionMenu {...rest} control={shown} />;
}

// The same model identity / variant picker can live behind a different trigger,
// including a plan's Build menu. Selection is owned by the caller.
export function ModelOptions({ control, hidden, onSelect, close }: {
  control: ConfigControl;
  hidden?: string[];
  onSelect: (value: string) => void;
  close: () => void;
}) {
  const shown = useMemo(() => visibleOptions(control.options, hidden, control.value), [control, hidden]);
  const families = useMemo(() => groupModels(shown), [shown]);
  const cur = families.find(f => f.variants.some(v => v.id === control.value));
  const curVar = cur?.variants.find(v => v.id === control.value);
  return <ModelPanel families={families} cur={cur} curVar={curVar} onSelect={onSelect} close={close} />;
}

// Every ACP uses the Devin-style model chip and panel. Separate ACP reasoning
// options join the footer, while Devin's embedded variants retain their wire IDs.
export function ModelControl({ control, hidden, reasoning = [], onSetReasoning, onSelect, onOpenChange }: OptionMenuProps & {
  hidden?: string[];
  reasoning?: ConfigControl[];
  onSetReasoning: (id: string, value: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const c = useMemo(() => ({ ...control, options: visibleOptions(control.options, hidden, control.value) }), [control, hidden]);
  const families = useMemo(() => groupModels(c.options), [c.options]);
  const cur = families.find(f => f.variants.some(v => v.id === c.value));
  const curVar = cur?.variants.find(v => v.id === c.value);
  // A Fusion pair is too long for the chip: it reads "Fusion" and keeps the pair in the tooltip and the panel
  const params = cur && curVar && !curVar.lead && (cur.efforts.length > 1 || curVar.effort || curVar.fast || curVar.long) ? variantLabel(curVar, cur, { standard: t('composer.standard') }) : undefined;
  const levels = reasoning.map(reasoningChip);
  // Provider identity stays in the expanded list; the chip reads as one model name.
  const meta = [params, ...levels].filter(Boolean).join(' ') || undefined;
  const title = [cur?.name ?? c.name, curVar?.lead ? fusionLabel(curVar) : meta].filter(Boolean).join(' ');
  return (
    <Popover.Root open={open} onOpenChange={setOpen} onOpenLifecycle={onOpenChange}>
      <Popover.Trigger render={
        <Chip narrow="text" title={title} meta={meta} icon={<ModelMark family={cur?.name ?? c.name} brand={cur?.brand} />}>
          {cur?.name ?? c.options.find(o => o.id === c.value)?.name ?? c.name}
        </Chip>
      } />
      <Popover.Portal><Popover.Positioner side="top" align="end" width="md"><Popover.Popup>
        <ModelPanel families={families} cur={cur} curVar={curVar} onSelect={onSelect} close={() => setOpen(false)}
          reasoning={reasoning} onSetReasoning={onSetReasoning} />
      </Popover.Popup></Popover.Positioner></Popover.Portal>
    </Popover.Root>
  );
}

// Agents without a model selector still use the same reasoning field and labels.
export function ReasoningControl({ control: c, onSelect, onOpenChange }: OptionMenuProps) {
  if (!reasoningVisible(c)) return null;
  return <Popover.Root onOpenLifecycle={onOpenChange}>
    <Popover.Trigger render={<Chip narrow="text" title={t('composer.effort')}>
      {reasoningChip(c) ?? t('composer.effort')}
    </Chip>} />
    <Popover.Portal><Popover.Positioner side="top" align="end" width="md"><Popover.Popup>
      <ReasoningParams control={c} onChange={onSelect} />
    </Popover.Popup></Popover.Positioner></Popover.Portal>
  </Popover.Root>;
}

interface ModelPanelProps {
  families: ModelFamily[];
  cur?: ModelFamily;
  curVar?: ModelVariant;
  onSelect: (value: string) => void;
  close: () => void;
  reasoning?: ConfigControl[];
  onSetReasoning?: (id: string, value: string) => void;
}

function ModelPanel({ families, cur, curVar, onSelect, close, reasoning = [], onSetReasoning }: ModelPanelProps) {
  const pickFamily = (key: string) => {
    const f = families.find(x => x.key === key);
    if (!f) return;
    // Into Fusion: the model in use becomes the lead when it is one (Claude Opus 5 High → Fusion (Claude Opus 5 High + …)); out of it: the lead's effort carries over
    const next = f.fusion
      ? findFusionVariant(f, { lead: curVar?.lead ?? cur?.name ?? '', effort: curVar?.effort ?? '', sidekick: curVar?.sidekick, fast: curVar?.fast ?? false })
      : curVar && findVariant(f, curVar.effort, curVar.fast, curVar.long);
    onSelect((next ?? f.variants[0]!).id);
    close();
  };
  const shown = reasoning.filter(reasoningVisible);
  const showParams = !!(cur && curVar && cur.variants.length > 1);
  const twoLine = families.some(f => f.source);
  return (
    <>
      <Command.Root items={families} value={cur ?? null} itemToStringLabel={f => [f.name, f.source].filter(Boolean).join(' ')}
        itemToStringValue={f => f.key} isItemEqualToValue={(a, b) => a.key === b.key}>
        <Command.Input visible={families.length >= SEARCH_FROM} />
        <Command.Empty />
        <Command.List searchable={families.length >= SEARCH_FROM}>
          {(f: ModelFamily) => <Command.Item key={f.key} value={f} title={f.description} onClick={() => pickFamily(f.key)} className={twoLine ? 'min-h-0 py-1.5' : undefined}>
            <OptionContent icon={<ModelMark family={f.name} brand={f.brand} />} description={f.source} checked={f === cur} checkSlot={!!cur}>{f.name}</OptionContent>
          </Command.Item>}
        </Command.List>
      </Command.Root>
      {(showParams || shown.length > 0) && <div className="mt-1 flex flex-col border-t border-line pt-1">
        {showParams && (cur!.fusion ? <FusionParams family={cur!} variant={curVar!} onSelect={onSelect} /> : <ModelParams family={cur!} variant={curVar!} onSelect={onSelect} />)}
        {shown.map(c => <ReasoningParams key={c.id} control={c} onChange={value => onSetReasoning?.(c.id, value)} />)}
      </div>}
    </>
  );
}

// Devin's Fusion pair as the four controls its own picker has: Lead (menu), Effort (pills of what that lead offers), Sidekick (menu),
// Fast (switch). Every change resolves to a real option through findFusionVariant, so a combination the agent doesn't offer lands on
// the nearest one instead of failing; a sidekick or Fast the current lead cannot take is disabled rather than hidden
function FusionParams({ family: f, variant: v, onSelect }: { family: ModelFamily; variant: ModelVariant; onSelect: (id: string) => void }) {
  const { leads, sidekicks } = f.fusion!;
  const lead = v.lead ?? leads[0] ?? '';
  const ofLead = f.variants.filter(x => x.lead === lead);
  const efforts = f.efforts.filter(e => ofLead.some(x => x.effort === e));
  const pick = (want: Partial<{ lead: string; effort: string; sidekick: string; fast: boolean }>) => {
    const hit = findFusionVariant(f, { lead, effort: v.effort, sidekick: v.sidekick, fast: v.fast, ...want });
    if (hit) onSelect(hit.id);
  };
  const offers = (sidekick: string) => ofLead.some(x => x.effort === v.effort && x.sidekick === sidekick);
  const canFast = ofLead.some(x => x.effort === v.effort && x.sidekick === v.sidekick && x.fast === !v.fast);
  return (
    <div className="flex flex-col">
      <SelectRow label={t('composer.lead')} options={leads.map(l => ({ value: l, label: l }))} value={lead} onChange={l => pick({ lead: l })} />
      {efforts.length > 1 && <EffortField options={efforts.map(e => ({ value: e, label: e || t('composer.standard') }))} value={v.effort} onChange={e => pick({ effort: e })} />}
      <SelectRow label={t('composer.sidekick')} options={sidekicks.map(s => ({ value: s, label: s, disabled: !offers(s) }))} value={v.sidekick ?? ''} onChange={s => pick({ sidekick: s })} />
      {f.hasFast && <SwitchRow label="Fast" checked={v.fast} disabled={!canFast} onChange={on => pick({ fast: on })} />}
    </div>
  );
}

// Params of the current family as a small form under the list: reasoning levels as radio pills (every level visible, one click), Fast / 1M as switches.
// A family whose only levels are Standard / Thinking gets a Thinking switch instead of two pills (what Cursor does). Every change applies immediately;
// a flag whose combination the agent doesn't offer is disabled rather than hidden, so the shape of the form doesn't jump between variants
function ModelParams({ family: f, variant: v, onSelect }: { family: ModelFamily; variant: ModelVariant; onSelect: (id: string) => void }) {
  const at = (effort: string, fast: boolean, long: boolean) => f.variants.find(x => x.effort === effort && x.fast === fast && x.long === long);
  const thinkingSwitch = f.efforts.length === 2 && f.efforts.includes('') && f.efforts.includes('Thinking');
  // Level changes keep Fast / 1M when that tier has them, otherwise drop them (findVariant's fallback order)
  const pickEffort = (e: string) => { const hit = findVariant(f, e, v.fast, v.long); if (hit) onSelect(hit.id); };
  const flip = (key: 'fast' | 'long') => { const hit = at(v.effort, key === 'fast' ? !v.fast : v.fast, key === 'long' ? !v.long : v.long); if (hit) onSelect(hit.id); };
  return (
    <div className="flex flex-col">
      {f.efforts.length > 1 && !thinkingSwitch && (
        <EffortField options={f.efforts.map(e => ({ value: e, label: e || t('composer.standard') }))} value={v.effort} onChange={pickEffort} />
      )}
      {thinkingSwitch && (
        <SwitchRow label="Thinking" checked={v.effort === 'Thinking'} disabled={!findVariant(f, v.effort === 'Thinking' ? '' : 'Thinking', v.fast, v.long)} onChange={on => pickEffort(on ? 'Thinking' : '')} />
      )}
      {f.hasFast && <SwitchRow label="Fast" checked={v.fast} disabled={!at(v.effort, !v.fast, v.long)} onChange={() => flip('fast')} />}
      {f.hasLong && <SwitchRow label="1M" checked={v.long} disabled={!at(v.effort, v.fast, !v.long)} onChange={() => flip('long')} />}
    </div>
  );
}

// Native thought_level: effort pills, plus a Thinking switch when the agent can turn it off.
function ReasoningParams({ control, onChange }: { control: ConfigControl; onChange: (value: string) => void }) {
  const p = presentReasoning(control);
  const turnOn = p.efforts.find(o => o.id === 'high')?.id ?? p.efforts[0]?.id ?? p.onId;
  return (
    <div className="flex flex-col">
      {p.offId && <SwitchRow label="Thinking" checked={!p.off} onChange={on => { if (on) { if (turnOn) onChange(turnOn); } else onChange(p.offId!); }} />}
      {p.efforts.length > 1 && (
        <EffortField options={p.efforts.map(o => ({ value: o.id, label: o.name }))} value={p.off ? '' : (p.value ?? '')} onChange={onChange} />
      )}
    </div>
  );
}

// Shared segmented field for both embedded variants and native thought_level.
function EffortField({ options, value, onChange, label = t('composer.effort') }: {
  options: { value: string; label: string }[];
  value: string;
  onChange: (value: string) => void;
  label?: string;
}) {
  return <div className={cn('flex min-h-row gap-2 px-2 py-1', options.length > 3 ? 'flex-col' : 'flex-wrap items-center')}>
    <span className="shrink-0 text-2 text-fg-2">{label}</span>
    <RadioPills label={label} options={options} value={value} onChange={onChange} />
  </div>;
}

// Flat configOption menu; long lists are searchable. Vendor marks only appear when at least one option has a known brand
// (Grok's monolithic model list) — a thought_level menu of "Low / High" stays text-only instead of earning letter tiles
function OptionMenu({ control: c, end, onSelect, onOpenChange }: OptionMenuProps) {
  const branded = c.options.some(o => optionBrand(o));
  const cur = c.options.find(o => o.id === c.value);
  const curIcon = branded && cur && optionBrand(cur) ? <ModelMark family={cur.name} brand={optionBrand(cur)} /> : undefined;
  const [open, setOpen] = useState(false);
  const trigger = <Chip narrow="text" title={c.name} icon={curIcon}>{cur?.name ?? c.name}</Chip>;
  const twoLine = c.options.some(o => o.description);
  const content = (o: ConfigControl['options'][number]) => <OptionContent
    icon={branded ? <ModelMark family={o.name} brand={optionBrand(o)} /> : undefined}
    description={o.description} checked={o.id === c.value} checkSlot={!!cur}>{o.name}</OptionContent>;
  if (c.options.length >= SEARCH_FROM) return <Popover.Root open={open} onOpenChange={setOpen} onOpenLifecycle={onOpenChange}>
    <Popover.Trigger render={trigger} />
    <Popover.Portal><Popover.Positioner side="top" align={end ? 'end' : 'start'} width="md"><Popover.Popup>
      <Command.Root items={c.options} value={cur ?? null} itemToStringValue={o => o.id} itemToStringLabel={o => [o.name, o.description].filter(Boolean).join(' ')}
        isItemEqualToValue={(a, b) => a.id === b.id}>
        <Command.Input /><Command.Empty />
        <Command.List searchable>{(o: ConfigControl['options'][number]) => <Command.Item key={o.id} value={o}
          className={twoLine ? 'min-h-0 py-1.5' : undefined} onClick={() => { onSelect(o.id); setOpen(false); }}>{content(o)}</Command.Item>}</Command.List>
      </Command.Root>
    </Popover.Popup></Popover.Positioner></Popover.Portal>
  </Popover.Root>;
  return <DropdownMenu.Root onOpenLifecycle={onOpenChange}>
    <DropdownMenu.Trigger render={trigger} />
    <DropdownMenu.Portal><DropdownMenu.Positioner side="top" align={end ? 'end' : 'start'} width="md"><DropdownMenu.Popup>
      <DropdownMenu.RadioGroup value={c.value} className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
        {c.options.map(o => <DropdownMenu.RadioItem key={o.id} value={o.id} onClick={() => onSelect(o.id)} className={twoLine ? 'min-h-0 py-1.5' : undefined}>{content(o)}</DropdownMenu.RadioItem>)}
      </DropdownMenu.RadioGroup>
    </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
  </DropdownMenu.Root>;
}
