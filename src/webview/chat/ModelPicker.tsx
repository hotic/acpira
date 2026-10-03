import { useMemo, useState, type ReactNode } from 'react';
import { Zap } from 'lucide-react';
import type { ConfigControl } from '@shared/transcript';
import { findFusionVariant, findVariant, fusionLabel, groupModels, optionBrand, visibleOptions, type ModelFamily, type ModelVariant } from '@shared/models';
import { fastOn, fastValue, isFastControl, isUltraLevel, modelConfigChip, presentReasoning, reasoningChip, reasoningVisible } from '@shared/composerControls';
import { t } from '../i18n';
import { Chip, ChipTag } from '../ui/Button';
import { HeaderToggle, SegmentScale, SelectRow, SwitchRow } from '../ui/Field';
import { cn } from '../ui/cn';
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

// Every ACP uses the Devin-style model chip and panel. Native reasoning and model
// parameters join the footer, while embedded variants retain their wire IDs.
export function ModelControl({ control, hidden, reasoning = [], modelConfig = [], hiddenConfig, onSetConfig, onSelect, onOpenChange }: OptionMenuProps & {
  hidden?: string[];
  reasoning?: ConfigControl[];
  modelConfig?: ConfigControl[];
  hiddenConfig?: Record<string, string[]>;
  onSetConfig: (id: string, value: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const c = useMemo(() => ({ ...control, options: visibleOptions(control.options, hidden, control.value) }), [control, hidden]);
  const families = useMemo(() => groupModels(c.options), [c.options]);
  const cur = families.find(f => f.variants.some(v => v.id === c.value));
  const curVar = cur?.variants.find(v => v.id === c.value);
  const tags = chipTags(cur, curVar, reasoning, modelConfig);
  // A Fusion pair is too long for the chip: it reads "Fusion" plus the lead's badges and keeps the pair in the tooltip and the panel
  const title = [cur?.name ?? c.name, curVar?.lead ? fusionLabel(curVar) : tags.map(tag => tag.label).join(' ')].filter(Boolean).join(' ');
  return (
    <Popover.Root open={open} onOpenChange={setOpen} onOpenLifecycle={onOpenChange}>
      <Popover.Trigger render={
        <Chip narrow="text" title={title} icon={<ModelMark family={cur?.name ?? c.name} brand={cur?.brand} />}
          tags={tags.length ? tags.map(tag => <ChipTag key={tag.label} tone={tag.ultra ? 'ultra' : undefined} icon={tag.fast ? <FastIcon /> : undefined}>{tag.label}</ChipTag>) : undefined}>
          {cur?.name ?? c.options.find(o => o.id === c.value)?.name ?? c.name}
        </Chip>
      } />
      <Popover.Portal><Popover.Positioner side="top" align="end" width="lg"><Popover.Popup palette="menu">
        <ModelPanel families={families} cur={cur} curVar={curVar} onSelect={onSelect} close={() => setOpen(false)}
          reasoning={reasoning} modelConfig={modelConfig} hiddenConfig={hiddenConfig} onSetConfig={onSetConfig} />
      </Popover.Popup></Popover.Positioner></Popover.Portal>
    </Popover.Root>
  );
}

interface ChipTagItem { label: string; ultra?: boolean; fast?: boolean }

// The chip's badges, in the order they give way when the toolbar is narrow: effort (or Ultra), then Fast, then the rest.
// Provider identity stays in the expanded list; the chip reads as one model name plus its parameters
function chipTags(cur: ModelFamily | undefined, curVar: ModelVariant | undefined, reasoning: ConfigControl[], modelConfig: ConfigControl[]): ChipTagItem[] {
  const tags: ChipTagItem[] = [];
  const fast: ChipTagItem = { label: 'Fast', fast: true };
  if (cur && curVar && (curVar.lead || cur.efforts.length > 1 || curVar.effort || curVar.fast || curVar.long)) {
    const effort = curVar.effort || (!curVar.lead && cur.efforts.length > 1 ? t('composer.standard') : '');
    if (effort) tags.push({ label: effort });
    if (curVar.fast) tags.push(fast);
    if (curVar.long) tags.push({ label: '1M' });
  }
  for (const control of reasoning) {
    const level = reasoningChip(control);
    if (level) tags.push({ label: level, ultra: isUltraLevel(level) });
  }
  for (const control of modelConfig) {
    const label = modelConfigChip(control);
    if (label) tags.push(isFastControl(control) ? fast : { label });
  }
  return tags;
}

function FastIcon() {
  return <Zap className="size-2.5 shrink-0 fill-current text-fast" strokeWidth={2.25} aria-hidden />;
}

// Agents without a model selector still use the same reasoning field and labels.
export function ReasoningControl({ control: c, onSelect, onOpenChange }: OptionMenuProps) {
  if (!reasoningVisible(c)) return null;
  return <Popover.Root onOpenLifecycle={onOpenChange}>
    <Popover.Trigger render={<Chip narrow="text" title={t('composer.effort')}>
      {reasoningChip(c) ?? t('composer.effort')}
    </Chip>} />
    <Popover.Portal><Popover.Positioner side="top" align="end" width="lg"><Popover.Popup palette="menu">
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
  modelConfig?: ConfigControl[];
  hiddenConfig?: Record<string, string[]>;
  onSetConfig?: (id: string, value: string) => void;
}

// One Fast toggle per panel, whatever its source: an embedded model-name variant, a Fusion pair, or a native model_config control
interface FastState { on: boolean; disabled: boolean; set: (on: boolean) => void }

function ModelPanel({ families, cur, curVar, onSelect, close, reasoning = [], modelConfig = [], hiddenConfig, onSetConfig }: ModelPanelProps) {
  const pickFamily = (key: string) => {
    const f = families.find(x => x.key === key);
    if (!f) return;
    // Into Fusion: the model in use becomes the lead when it is one (Claude Opus 5 High → Fusion (Claude Opus 5 High + …)); out of it: the lead's effort carries over.
    // 1M variants are not offered by the panel, so a family switch lands on the standard-context variant
    const next = f.fusion
      ? findFusionVariant(f, { lead: curVar?.lead ?? cur?.name ?? '', effort: curVar?.effort ?? '', sidekick: curVar?.sidekick, fast: curVar?.fast ?? false })
      : curVar && findVariant(f, curVar.effort, curVar.fast, false);
    onSelect((next ?? f.variants[0]!).id);
    close();
  };
  const shown = reasoning.filter(reasoningVisible);
  const showParams = !!(cur && curVar && cur.variants.length > 1);
  const twoLine = families.some(f => f.source);
  const nativeFast = modelConfig.find(isFastControl);
  const fast: FastState | undefined = showParams && cur!.hasFast
    ? (cur!.fusion ? fusionFast(cur!, curVar!, onSelect) : variantFast(cur!, curVar!, onSelect))
    : nativeFast && nativeFastState(nativeFast, hiddenConfig?.[nativeFast.id], value => onSetConfig?.(nativeFast.id, value));
  const usedNative = fast && !(showParams && cur!.hasFast) ? nativeFast : undefined;
  // The Fast toggle sits in the first effort header; with no scale to sit on it stays a switch row
  const embeddedScale = showParams && (cur!.fusion ? fusionEfforts(cur!, curVar!).length > 1 : variantScale(cur!));
  const reasoningScale = shown.findIndex(control => presentReasoning(control).efforts.length > 1);
  const owner = !fast ? undefined : embeddedScale ? 'embedded' : reasoningScale >= 0 ? 'reasoning' : 'row';
  const toggle = fast && <HeaderToggle label="Fast" icon={<Zap className={cn('size-3 shrink-0', fast.on && 'fill-current text-fast')} strokeWidth={2} aria-hidden />}
    pressed={fast.on} disabled={fast.disabled} onChange={fast.set} />;
  const rest = modelConfig.filter(control => control !== usedNative);
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
      {(showParams || shown.length > 0 || modelConfig.length > 0) && <div className="mt-1 flex flex-col gap-0.5 border-t border-line pt-1.5">
        {showParams && (cur!.fusion
          ? <FusionParams family={cur!} variant={curVar!} onSelect={onSelect} actions={owner === 'embedded' ? toggle : undefined} />
          : <ModelParams family={cur!} variant={curVar!} onSelect={onSelect} actions={owner === 'embedded' ? toggle : undefined} />)}
        {shown.map((control, i) => <ReasoningParams key={control.id} control={control} onChange={value => onSetConfig?.(control.id, value)}
          actions={owner === 'reasoning' && i === reasoningScale ? toggle : undefined} />)}
        {owner === 'row' && <SwitchRow label="Fast" checked={fast!.on} disabled={fast!.disabled} onChange={fast!.set} />}
        {rest.map(control => <ModelConfigParams key={control.id} control={control} hidden={hiddenConfig?.[control.id]} onChange={value => onSetConfig?.(control.id, value)} />)}
      </div>}
    </>
  );
}

// Every native Fast shape (Devin's Standard / Fast select, an adapter boolean) as one toggle; hidden values stay respected
function nativeFastState(control: ConfigControl, hidden: string[] | undefined, onChange: (value: string) => void): FastState {
  const on = fastOn(control);
  const next = fastValue(control, !on);
  return { on, disabled: !visibleOptions(control.options, hidden, control.value).some(option => option.id === next), set: () => onChange(next) };
}

// Embedded Fast: the same effort with Fast flipped, standard context first (1M is not offered), else the current context
function variantFast(f: ModelFamily, v: ModelVariant, onSelect: (id: string) => void): FastState {
  const at = (long: boolean) => f.variants.find(x => x.effort === v.effort && x.fast === !v.fast && x.long === long);
  const hit = at(false) ?? at(v.long);
  return { on: v.fast, disabled: !hit, set: () => { if (hit) onSelect(hit.id); } };
}

// Fusion Fast is pair-level: offered when the same lead, effort and sidekick exist with Fast flipped
function fusionFast(f: ModelFamily, v: ModelVariant, onSelect: (id: string) => void): FastState {
  const lead = v.lead ?? f.fusion?.leads[0] ?? '';
  const can = f.variants.some(x => x.lead === lead && x.effort === v.effort && x.sidekick === v.sidekick && x.fast === !v.fast);
  return { on: v.fast, disabled: !can, set: on => {
    const hit = findFusionVariant(f, { lead, effort: v.effort, sidekick: v.sidekick, fast: on });
    if (hit) onSelect(hit.id);
  } };
}

// A family whose only levels are Standard / Thinking gets a Thinking switch instead of two segments (what Cursor does)
const thinkingOnly = (f: ModelFamily) => f.efforts.length === 2 && f.efforts.includes('') && f.efforts.includes('Thinking');
const variantScale = (f: ModelFamily) => f.efforts.length > 1 && !thinkingOnly(f);
const fusionEfforts = (f: ModelFamily, v: ModelVariant) => {
  const lead = v.lead ?? f.fusion?.leads[0] ?? '';
  return f.efforts.filter(e => f.variants.some(x => x.lead === lead && x.effort === e));
};

// Category chooses placement; unfamiliar model parameters retain their advertised labels and wire values.
// Fast never reaches here when the panel already shows its toggle
function ModelConfigParams({ control, hidden, onChange }: { control: ConfigControl; hidden?: string[]; onChange: (value: string) => void }) {
  const options = visibleOptions(control.options, hidden, control.value);
  if (isFastControl(control)) {
    const fast = nativeFastState(control, hidden, onChange);
    return <SwitchRow label="Fast" checked={fast.on} disabled={fast.disabled} onChange={fast.set} />;
  }
  // An ACP boolean arrives as a synthetic Off/On pair; the panel shows it as the switch it really is
  if (control.type === 'boolean') {
    return <SwitchRow label={control.name} checked={control.value === 'true'} onChange={on => onChange(on ? 'true' : 'false')} />;
  }
  return <SelectRow label={control.name} options={options.map(option => ({ value: option.id, label: option.name }))} value={control.value ?? ''} onChange={onChange} />;
}

// Devin's Fusion pair as its own picker's controls: Lead (menu), Effort (segments of what that lead offers), Sidekick (menu); the pair's
// Fast toggle rides in the effort header. Every change resolves to a real option through findFusionVariant, so a combination the agent
// doesn't offer lands on the nearest one instead of failing; a sidekick the current lead cannot take is disabled rather than hidden
function FusionParams({ family: f, variant: v, onSelect, actions }: { family: ModelFamily; variant: ModelVariant; onSelect: (id: string) => void; actions?: ReactNode }) {
  const { leads, sidekicks } = f.fusion!;
  const lead = v.lead ?? leads[0] ?? '';
  const ofLead = f.variants.filter(x => x.lead === lead);
  const efforts = fusionEfforts(f, v);
  const pick = (want: Partial<{ lead: string; effort: string; sidekick: string; fast: boolean }>) => {
    const hit = findFusionVariant(f, { lead, effort: v.effort, sidekick: v.sidekick, fast: v.fast, ...want });
    if (hit) onSelect(hit.id);
  };
  const offers = (sidekick: string) => ofLead.some(x => x.effort === v.effort && x.sidekick === sidekick);
  return (
    <div className="flex flex-col">
      <SelectRow label={t('composer.lead')} options={leads.map(l => ({ value: l, label: l }))} value={lead} onChange={l => pick({ lead: l })} />
      {efforts.length > 1 && <EffortField options={efforts.map(e => ({ value: e, label: e || t('composer.standard') }))} value={v.effort} onChange={e => pick({ effort: e })} actions={actions} />}
      <SelectRow label={t('composer.sidekick')} options={sidekicks.map(s => ({ value: s, label: s, disabled: !offers(s) }))} value={v.sidekick ?? ''} onChange={s => pick({ sidekick: s })} />
    </div>
  );
}

// Params of the current family as a small form under the list: reasoning levels as one segmented scale (every level spelled out, one click),
// Fast as a toggle in its header. A lone Standard / Thinking pair is a Thinking switch. Every change applies immediately; 1M variants are
// not offered (a level change keeps the standard context), and a Fast the current level lacks is disabled rather than hidden
function ModelParams({ family: f, variant: v, onSelect, actions }: { family: ModelFamily; variant: ModelVariant; onSelect: (id: string) => void; actions?: ReactNode }) {
  const pickEffort = (e: string) => { const hit = findVariant(f, e, v.fast, false); if (hit) onSelect(hit.id); };
  return (
    <div className="flex flex-col">
      {variantScale(f) && (
        <EffortField options={f.efforts.map(e => ({ value: e, label: e || t('composer.standard') }))} value={v.effort} onChange={pickEffort} actions={actions} />
      )}
      {thinkingOnly(f) && (
        <SwitchRow label="Thinking" checked={v.effort === 'Thinking'} disabled={!findVariant(f, v.effort === 'Thinking' ? '' : 'Thinking', v.fast, false)} onChange={on => pickEffort(on ? 'Thinking' : '')} />
      )}
    </div>
  );
}

// Native thought_level: effort segments (Codex's Ultra tinted as the last one), plus a Thinking switch when the agent can turn it off.
function ReasoningParams({ control, onChange, actions }: { control: ConfigControl; onChange: (value: string) => void; actions?: ReactNode }) {
  const p = presentReasoning(control);
  const turnOn = p.efforts.find(o => o.id === 'high')?.id ?? p.efforts[0]?.id ?? p.onId;
  return (
    <div className="flex flex-col">
      {p.offId && <SwitchRow label="Thinking" checked={!p.off} onChange={on => { if (on) { if (turnOn) onChange(turnOn); } else onChange(p.offId!); }} />}
      {p.efforts.length > 1 && (
        <EffortField options={p.efforts.map(o => ({ value: o.id, label: o.name, ultra: isUltraLevel(o.name) }))} value={p.off ? '' : (p.value ?? '')} onChange={onChange} actions={actions} />
      )}
    </div>
  );
}

// Shared ordinal field for embedded variants, Fusion leads and native thought_level: one segmented scale for any level count
function EffortField({ options, value, onChange, actions, label = t('composer.effort') }: {
  options: { value: string; label: string; ultra?: boolean }[];
  value: string;
  onChange: (value: string) => void;
  actions?: ReactNode;
  label?: string;
}) {
  return <SegmentScale label={label} options={options} value={value} onChange={onChange} actions={actions} />;
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
    <Popover.Portal><Popover.Positioner side="top" align={end ? 'end' : 'start'} width="md"><Popover.Popup palette="menu">
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
