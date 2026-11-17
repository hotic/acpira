import { Fragment, useCallback, useLayoutEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from 'react';
import { Ban, Brain, Check, ChevronRight, Plus, Zap } from 'lucide-react';
import type { ConfigControl } from '@acpira/shared/transcript';
import { shapeFor, type ModelMemory, type ModelShapes } from '@acpira/shared/modelShapes';
import { findFusionVariant, findVariant, fusionLabel, fusionSidekick, groupModels, modelBrand, optionBrand, parseModelName, visibleOptions, type ModelFamily, type ModelVariant } from '@acpira/shared/models';
import { chipTags, composerControls, fastOn, fastValue, isFastControl, isUltraLevel, levelShort, NO_SIDEKICK, presentReasoning, reasoningChip, reasoningVisible, sidekickChannel, sidekickClis, type SidekickCli } from '@acpira/shared/composerControls';
import { t } from '../i18n';
import { Chip, ChipTag, CORNER_CHIP } from '../ui/Button';
import { SegmentScale, SelectRow, SwitchRow } from '../ui/Field';
import { cn } from '../ui/cn';
import { Popover } from '../ui/Popover';
import { DropdownMenu } from '../ui/DropdownMenu';
import { Command, SEARCH_FROM } from '../ui/Command';
import { RadioGroup } from '../ui/RadioGroup';
import { OptionContent } from '../ui/Panel';
import { optionClass } from '../ui/DropdownMenu';
import { AgentMark } from './AgentMark';
import { ModelMark } from './ModelMark';
import { PairSwitch, PairViews, type PairRole } from './SidekickPair';
import { brandStyle } from './subagents/SidekickRelay';
import { useFitStage, type SqueezeLevel } from './useFitStage';
import { Collapsible } from '../ui/Collapsible';
import { cssTimeVar } from '../ui/cssTime';
import { useScrollFade } from '../ui/useScrollFade';


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
export function ModelControl({ control, hidden, reasoning = [], modelConfig = [], sidekick, sidekickEffort, sidekickFast, sidekickLive, sidekickShapes, hiddenConfig, onSetConfig, memory, shapes, onSqueeze, onOpenChange, onConfigure, toggleRef }: OptionMenuProps & {
  hidden?: string[];
  reasoning?: ConfigControl[];
  modelConfig?: ConfigControl[];
  // The built-in agent's sidekick select and its effort: the panel's second side (PairSwitch); a picked one is spelled out on the chip
  sidekick?: ConfigControl;
  sidekickEffort?: ConfigControl;
  // Its Fast switch, while its model has a fast tier: the toggle on its side of the panel
  sidekickFast?: ConfigControl;
  // The sidekick is working a round (chat/subagents/SidekickRelay.tsx)
  sidekickLive?: boolean;
  // What each sidekick option runs with once picked (SessionView.sidekickShapes)
  sidekickShapes?: ModelShapes;
  hiddenConfig?: Record<string, string[]>;
  // `also`: parameters to set once the pick has landed (a model picked together with its level)
  onSetConfig: (id: string, value: string, also?: Record<string, string>) => void;
  // What each model was last run with and the parameters it came with (engine prefs): the rows' own levels
  memory?: ModelMemory;
  shapes?: ModelShapes;
  // Asks the toolbar to give way while the chip cannot keep its name, then its last form (useFitStage squeeze)
  onSqueeze?: (level: SqueezeLevel) => void;
  // Opens the current agent's model settings from the shared header above both sides
  onConfigure?: () => void;
  // Imperative toggle for the ⌘⇧M / Ctrl+Shift+M model picker shortcut
  toggleRef?: RefObject<(() => void) | undefined>;
}) {
  const [open, setOpen] = useState(false);
  if (toggleRef) toggleRef.current = () => setOpen(o => !o);
  const c = useMemo(() => ({ ...control, options: visibleOptions(control.options, hidden, control.value) }), [control, hidden]);
  const families = useMemo(() => groupModels(c.options), [c.options]);
  const cur = families.find(f => f.variants.some(v => v.id === c.value));
  const curVar = cur?.variants.find(v => v.id === c.value);
  // A Fusion pair is a lead and a sidekick of Devin's own: the chip and the panel spell it like a picked sidekick, and the
  // host's sidekick is not offered beside it (the engine sets its select aside too, host_sidekick.rs)
  const fusion = curVar && fusionSidekick(curVar);
  const own = !fusion;
  // The sidekick offers the models the model list does: the ones hidden there stay out (unless picked)
  const sk = useMemo(() => (own && sidekick ? { ...sidekick, options: visibleOptions(sidekick.options, hidden, sidekick.value) } : undefined), [own, sidekick, hidden]);
  const tags = chipTags(cur, curVar, reasoning, modelConfig, t('composer.standard'));
  // A picked sidekick is spelled out after the lead, `Opus 5.5 [High] + GPT-6 Luna [Medium]`, stepping down (CHIP_STAGES)
  // when the toolbar has no room; without it the pair was only visible with the panel open (reported 2026-10-26)
  const skPicked = own ? sidekick?.options.find(o => o.id === sidekick.value && o.id !== NO_SIDEKICK) : undefined;
  const skLevel = skPicked && sidekickEffort ? reasoningChip(sidekickEffort) : undefined;
  const skFast = !!skPicked && !!sidekickFast && fastOn(sidekickFast);
  const tail = fusion
    ? { name: fusion.name, brand: modelBrand(fusion.name), level: fusion.effort || undefined, fast: false }
    : skPicked && { name: skPicked.name, brand: optionBrand(skPicked), level: skLevel, fast: skFast };
  const skLabel = tail && [tail.name, tail.level, tail.fast ? 'Fast' : undefined].filter(Boolean).join(' ');
  const leadName = curVar?.lead ?? cur?.name ?? c.name;
  const lead = [leadName, tags.map(tag => tag.label).join(' ')].filter(Boolean).join(' ');
  const pair = skLabel ? `${lead} + ${skLabel}` : lead;
  const title = fusion ? `${cur!.name} (${pair})` : pair;
  // A single model steps down too (its level to the short form first), not only a pair: it used to keep its full width and
  // run past the send button in a narrow window. Below the compact form the toolbar folds its other chips first (onSqueeze)
  // The title spells every form's text: a pick that changes it fits the chip again before it paints
  const fit = useFitStage<HTMLButtonElement, HTMLSpanElement>(true, onSqueeze && { stage: 1, set: onSqueeze }, title);
  // The chip follows every pick at once, also while the panel is open (effort / Fast keep it open): it used to hold its
  // content until close, which read as a pick that had not landed (reported 2026-10-16). The panel is end-aligned to the
  // chip's right edge, so a chip resizing leftwards does not move the panel
  const chip = { title, family: leadName, brand: fusion ? undefined : cur?.brand, tags, name: fusion ? leadName : cur?.name ?? c.options.find(o => o.id === c.value)?.name ?? c.name };
  const at = (stage: number, ref?: typeof fit.item) => {
    const on = CHIP_STAGES[stage]!;
    return <Chip ref={ref} data-land="models" data-part={ref ? 'composer-model' : undefined} narrow="text" title={chip.title} caret={on.caret} data-compact={on.compact ? '' : undefined} icon={<ModelMark family={chip.family} brand={chip.brand} />}
      trail={tail && <SidekickTail live={!!sidekickLive} name={tail.name} brand={tail.brand} named={on.names} level={on.levels ? tail.level : undefined} fast={on.levels && tail.fast} />}
      tags={on.levels && chip.tags.length ? chip.tags.map((tag, i) => <ChipParam key={i} tag={tag} />) : undefined}>
      {on.names ? chip.name : null}
    </Chip>;
  };
  // The glyph sits in the rows' check column: the chip's own padding hangs past the line's inset
  const add = onConfigure && <Chip caret={false} className={cn('-mr-1.5 shrink-0', CORNER_CHIP)} icon={<Plus strokeWidth={1.75} />}
    title={t('settings.providers.addModel')} aria-label={t('settings.providers.addModel')}
    onClick={() => { setOpen(false); onConfigure(); }}>{null}</Chip>;
  return (<>
    <Popover.Root open={open} onOpenChange={setOpen} onOpenLifecycle={onOpenChange}>
      <Popover.Trigger render={at(fit.stage, fit.item)} />
      <Popover.Portal><Popover.Positioner side="top" align="end" width="lg"><Popover.Popup palette="menu">
        {/* Adding models belongs to the agent, not the currently selected side: a bare + at the end of the model search,
            or on a head line of its own for a lone model, which lists no search */}
        {add && !modelSearch(families, true) && <div className="mb-1 flex shrink-0 items-center justify-between px-2 pt-0.5">
          <span className="text-3 text-fg-3">{t('settings.agentTab.models')}</span>
          {add}
        </div>}
        <ModelPanel families={families} cur={cur} curVar={curVar} onSelect={(value, also) => onSetConfig(c.id, value, also)} close={() => setOpen(false)} fold add={add}
          model={c} memory={memory} shapes={shapes}
          reasoning={reasoning} modelConfig={modelConfig} sidekick={sk} sidekickEffort={sidekickEffort} sidekickFast={sidekickFast} sidekickShapes={sidekickShapes} hiddenConfig={hiddenConfig} onSetConfig={onSetConfig} />
      </Popover.Popup></Popover.Positioner></Popover.Portal>
    </Popover.Root>
    {/* Every stage laid out once, out of sight and out of the row's flow, for useFitStage to measure */}
    <span inert aria-hidden className="pointer-events-none invisible absolute size-0 overflow-hidden">
      <span ref={fit.box} className="flex w-max flex-col items-start">{CHIP_STAGES.map((_, i) => <Fragment key={i}>{at(i)}</Fragment>)}</span>
    </span>
  </>);
}

// How the chip steps down, one step per stage while the toolbar is short of room. With a sidekick the two sides always step
// together (both names or neither, both levels in the same form): both levels to their short form, then the caret, then both
// names (the marks stay), then both levels
const CHIP_STAGES = [
  { compact: false, caret: true, names: true, levels: true },
  { compact: true, caret: true, names: true, levels: true },
  { compact: true, caret: false, names: true, levels: true },
  { compact: true, caret: false, names: false, levels: true },
  { compact: true, caret: false, names: false, levels: false },
];

// A parameter on the model chip, read as part of the name: the level as faint text a word space after it, Fast as its
// bolt alone. No box and no motion: a pick changes the words in place. The short form takes over below the sm container
// tier and on a chip stepping down to fit (`data-compact`, useFitStage)
function ChipParam({ tag }: { tag: { label: string; short?: string; fast?: boolean; ultra?: boolean } }) {
  if (tag.fast) return <FastIcon />;
  const compact = tag.short !== undefined;
  return <span className={cn('shrink-0', tag.ultra ? 'text-ultra' : 'text-fg-3')}>
    <span className={cn(compact && '@max-sm:hidden group-data-[compact]/chip:hidden')}>{tag.label}</span>
    {tag.short && <span className="hidden @max-sm:inline group-data-[compact]/chip:inline">{tag.short}</span>}
  </span>;
}

// The sidekick's part of the model chip: + and its mark in the icon tone, then its name and level like the lead's.
// While it works a round its mark takes its vendor's colour with a breathing dot and its name the chip's strong text
function SidekickTail({ live, name, brand, named, level, fast }: { live: boolean; name: string; brand?: string; named: boolean; level?: string; fast?: boolean }) {
  return (
    <span className="sidekick-tail flex shrink-0" style={brandStyle(brand)} data-live={live ? '' : undefined}>
      <span className="flex items-center gap-1 whitespace-nowrap">
        <span className="text-fg-3">+</span>
        <span className="flex shrink-0 items-center gap-1">
          <span className="sidekick-tail-mark relative flex shrink-0 items-center [&_svg]:size-icon"><ModelMark family={name} brand={brand} colored={live} /><i className="sidekick-pulse" aria-hidden /></span>
          {named && <span className="sidekick-tail-name whitespace-nowrap">{name}</span>}
        </span>
        {level && <ChipParam tag={{ label: level, short: levelShort(level), ultra: isUltraLevel(level) }} />}
        {fast && <FastIcon />}
      </span>
    </span>
  );
}

function FastIcon() {
  return <Zap className="size-3 shrink-0 fill-current text-fast" strokeWidth={2} aria-hidden />;
}

// Agents without a model selector still use the same reasoning field and labels.
export function ReasoningControl({ control: c, onSelect, onOpenChange }: OptionMenuProps) {
  // Shows the live level, also while the panel is open, like the model chip
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

// The composer's panel always offers the model search once there is a choice to make, so its add button keeps one place;
// other model lists search from SEARCH_FROM like any menu
function modelSearch(families: ModelFamily[], fold?: boolean) {
  return fold ? families.length > 1 : families.length >= SEARCH_FROM;
}

interface ModelPanelProps {
  families: ModelFamily[];
  cur?: ModelFamily;
  curVar?: ModelVariant;
  onSelect: (value: string, also?: Record<string, string>) => void;
  close: () => void;
  // The model control with what the engine remembers of its models, for the rows' own levels
  model?: ConfigControl;
  memory?: ModelMemory;
  shapes?: ModelShapes;
  reasoning?: ConfigControl[];
  modelConfig?: ConfigControl[];
  sidekick?: ConfigControl;
  // The add-model button, placed at the end of the model search when the list shows one
  add?: ReactNode;
  sidekickEffort?: ConfigControl;
  sidekickFast?: ConfigControl;
  // What each sidekick option runs with once picked (SessionView.sidekickShapes)
  sidekickShapes?: ModelShapes;
  hiddenConfig?: Record<string, string[]>;
  onSetConfig?: (id: string, value: string, also?: Record<string, string>) => void;
  // The composer's panel, where a lone model offers no sidekick and its effort fits one row (a plan's Build menu keeps the
  // stacked form)
  fold?: boolean;
}

// One Fast toggle per panel, whatever its source: an embedded model-name variant, a Fusion pair, or a native model_config control
interface FastState { on: boolean; disabled: boolean; set: (on: boolean) => void }

// Fast as a switch row under the effort scale: the lead's, and the sidekick's in the same place on its side. Its bolt
// sits in the rows' icon column and lights with the switch
function FastRow({ fast }: { fast: FastState }) {
  return <SwitchRow label="Fast" icon={<Zap className={cn(fast.on && 'fill-current text-fast')} strokeWidth={1.75} aria-hidden />}
    checked={fast.on} disabled={fast.disabled} onChange={fast.set} />;
}
const thinkingIcon = <Brain strokeWidth={1.75} aria-hidden />;

function ModelPanel(props: ModelPanelProps) {
  const { cur, curVar, reasoning = [], modelConfig = [], sidekick, sidekickEffort, sidekickFast, onSetConfig, onSelect, fold } = props;
  // Every open starts on the lead
  const [role, setRole] = useState<PairRole>('lead');
  // A Fusion pair in the composer switches between its own two sides: the model list, and the sidekicks Devin pairs with
  // the lead in use
  const fusion = fold && curVar ? fusionSidekick(curVar) : undefined;
  if (fusion && cur?.fusion && curVar?.lead) {
    const sidekicks = cur.fusion.sidekicks.filter(s => cur.variants.some(v => v.lead === curVar.lead && v.sidekick === s));
    const leadLevel = chipTags(cur, curVar, reasoning, modelConfig, t('composer.standard')).find(tag => !tag.fast)?.label;
    return <>
      <PairViews role={role} lead={<LeadPanel {...props} />} rows={sidekicks.length}
        sidekick={<FusionSidekicks family={cur} variant={curVar} sidekicks={sidekicks} onSelect={id => onSelect(id)} />} />
      <PairSwitch role={role} onRole={setRole}
        lead={{ name: curVar.lead, brand: modelBrand(curVar.lead), level: leadLevel }}
        sidekick={{ name: fusion.name, brand: modelBrand(fusion.name), level: fusion.effort || undefined }} />
    </>;
  }
  if (!sidekick) return <LeadPanel {...props} />;
  const picked = sidekick.options.find(o => o.id === sidekick.value && o.id !== NO_SIDEKICK);
  // A lone model offers no sidekick: the panel is its parameters alone. One still picked from a longer list keeps the pair,
  // so it can be switched off
  if (isLone(props) && !picked) return <LeadPanel {...props} />;
  const leadLevel = chipTags(cur, curVar, reasoning, modelConfig, t('composer.standard')).find(tag => !tag.fast)?.label;
  return <>
    <PairViews role={role} lead={<LeadPanel {...props} />} rows={sidekick.options.length + new Set(sidekick.options.map(o => o.description).filter(Boolean)).size}
      sidekick={<SidekickPanel control={sidekick} effort={sidekickEffort} fast={sidekickFast} shapes={props.sidekickShapes} onSetConfig={(id, value, also) => onSetConfig?.(id, value, also)} />} />
    {/* Under the lists, next to the chip it spells out: the search line stays the panel's top edge on either side */}
    <PairSwitch role={role} onRole={setRole}
      lead={{ name: cur?.name, brand: cur?.brand, level: leadLevel }}
      sidekick={{ name: picked?.name, brand: picked && optionBrand(picked), level: picked && sidekickEffort ? reasoningChip(sidekickEffort) : undefined }} />
  </>;
}

// One model with something to set, in the composer's panel
function isLone({ families, cur, curVar, reasoning = [], modelConfig = [], fold }: ModelPanelProps) {
  const params = !!(cur && curVar && cur.variants.length > 1) || reasoning.some(reasoningVisible) || modelConfig.length > 0;
  return !!fold && params && families.length === 1 && families[0] === cur;
}

// What a model row stands for: the variant a click on it picks and the parameters it would run with. The current model
// reads the live controls; another one reads the shape the engine remembers of it, or, never seen yet, borrows the live
// reasoning levels as a guess (`guessed`: menu only, no level on the row). A guessed level the model turns out not to take
// is dropped by the engine once the switch has landed (protocol-gotchas: queued picks validate after the model answer)
// `recent`: the variant is one this model last ran, not one carried over from the model in use
interface FamilyView { variant: ModelVariant; reasoning: ConfigControl[]; modelConfig: ConfigControl[]; live: boolean; guessed?: boolean; recent?: boolean; picks?: Record<string, string> }

function familyView(f: ModelFamily, { cur, curVar, reasoning = [], modelConfig = [], model, memory, shapes }: ModelPanelProps): FamilyView {
  if (f === cur && curVar) return { variant: curVar, reasoning: reasoning.filter(reasoningVisible), modelConfig, live: true };
  // The variant last used, else the one a family switch lands on. Into Fusion: the model in use becomes the lead when it
  // is one (Claude Opus 5 High → Fusion (Claude Opus 5 High + …)); out of it: the lead's effort carries over. 1M variants
  // are not offered by the panel, so a family switch lands on the standard-context variant
  const recent = memory?.recent.map(id => f.variants.find(v => v.id === id)).find(Boolean);
  const carried = f.fusion
    ? findFusionVariant(f, { lead: curVar?.lead ?? cur?.name ?? '', effort: curVar?.effort ?? '', sidekick: curVar?.sidekick, fast: curVar?.fast ?? false })
    : curVar && findVariant(f, curVar.effort, curVar.fast, false);
  const variant = recent ?? carried ?? f.variants[0]!;
  const picks = memory?.picks[variant.id];
  const known = model ? shapeFor(shapes, model, variant.id) : undefined;
  const shape = known ?? reasoning;
  // The value a switch lands on: the model's own remembered pick, else the live value carried over, else the shape's
  const live = new Map([...reasoning, ...modelConfig].map(c => [c.id, c.value]));
  const landed = (c: ConfigControl) => [picks?.[c.id], live.get(c.id)].find(v => v !== undefined && c.options.some(o => o.id === v));
  const kept = composerControls(shape.map(c => { const value = landed(c); return value === undefined ? c : { ...c, value }; }));
  return { variant, reasoning: kept.reasoning.filter(reasoningVisible), modelConfig: kept.modelConfig, live: false, guessed: !known, recent: !!recent, picks };
}

const viewHasParams = (f: ModelFamily, v: FamilyView) => f.variants.length > 1 || v.reasoning.length > 0 || v.modelConfig.length > 0;

// A row's parameters after its name, read as one phrase with it: the level faint on the name's baseline, Fast as its bolt
function RowParams({ family: f, view }: { family: ModelFamily; view: FamilyView }) {
  if (f.fusion) {
    // The pair in use runs its lead at the agent's own level select when there is one (chipTags), not the one in its name
    const level = view.live ? view.reasoning.map(reasoningChip).find(Boolean) : undefined;
    return <span className="min-w-0 truncate text-3 text-fg-3">{fusionLabel(level ? { ...view.variant, effort: level } : view.variant)}</span>;
  }
  // Another model's row spells out only its own settings: a level carried over from the model in use (what a switch would
  // land on) stays in the menu, else every never-picked row would follow the live effort. A guessed shape has none
  const own = (c: ConfigControl) => view.live || (!view.guessed && view.picks?.[c.id] === c.value);
  const tags = chipTags(f, view.live || view.recent ? view.variant : undefined, view.reasoning.filter(own), view.modelConfig.filter(own), t('composer.standard'));
  if (!tags.length) return null;
  return <span className="flex shrink-0 items-baseline gap-0.5 text-3 text-fg-3">
    {tags.map((tag, i) => (tag.fast
      ? <Zap key={i} className="inline size-3 shrink-0 self-center fill-current text-fast" strokeWidth={2} aria-label="Fast" />
      : <span key={i} className={tag.ultra ? 'text-ultra' : undefined}>{tag.label}</span>))}
  </span>;
}

// The end of a model row: the check of the current model, and where the row has parameters the arrow into them, shown
// while the row is highlighted or its menu is open. A span, since the row itself is the button
function RowTrail({ checked, label, onOpen }: { checked: boolean; label?: string; onOpen?: (row: HTMLElement) => void }) {
  // The current row keeps its check under the keyboard's highlight (the panel opens on it); only the pointer swaps it
  const check = <Check className={cn('size-icon shrink-0 text-fg-1', onOpen && 'group-hover/row:hidden group-data-[params]/row:hidden')} strokeWidth={2} />;
  if (!onOpen) return checked ? check : <span className="size-icon shrink-0" aria-hidden />;
  return (
    <span role="button" tabIndex={-1} title={label} aria-label={label}
      onClick={e => { e.preventDefault(); e.stopPropagation(); const row = e.currentTarget.closest<HTMLElement>('[data-family]'); if (row) onOpen(row); }}
      className="-mx-1 flex h-row-trail shrink-0 items-center rounded-sm px-1 text-fg-3 hover:bg-active hover:text-fg-1">
      {checked && check}
      <ChevronRight strokeWidth={1.75} className={cn('size-icon shrink-0 group-data-[params]/row:block group-data-[params]/row:opacity-100',
        checked ? 'hidden group-hover/row:block' : 'opacity-0 group-data-[highlighted]/row:opacity-100')} />
    </span>
  );
}

// A model row's own menu, beside the row: its levels as a list, then its switches. Opened from the row's arrow or with
// the right arrow key; the left arrow key or Escape goes back to the list
function ParamsMenu({ anchor, onClose, children }: { anchor: HTMLElement; onClose: () => void; children: ReactNode }) {
  return (
    <Popover.Root open onOpenChange={next => { if (!next) onClose(); }}>
      {/* The menu rides along with its row while the list scrolls and hides once the list has clipped the row away
          (Floating UI's hide middleware sets data-anchor-hidden), back when the row scrolls into view again */}
      <Popover.Portal><Popover.Positioner anchor={anchor} side="right" align="end" width="sm" className="data-[anchor-hidden]:invisible">
        <Popover.Popup palette="menu" initialFocus aria-label={t('composer.effort')}
          onKeyDown={e => {
            if (e.key !== 'ArrowLeft' || (e.target as HTMLElement).closest('input, [role="menu"], [role="listbox"]')) return;
            e.preventDefault();
            onClose();
          }}>
          {children}
        </Popover.Popup>
      </Popover.Positioner></Popover.Portal>
    </Popover.Root>
  );
}

// An ordinal single-select as rows, the current level checked: the levels in a model row's own menu
function LevelList({ options, value, onChange }: { options: { value: string; label: string; ultra?: boolean }[]; value: string; onChange: (value: string) => void }) {
  return (
    <RadioGroup.Root value={value} onValueChange={v => onChange(v as string)} aria-label={t('composer.effort')} className="flex flex-col">
      {options.map(o => <RadioGroup.Item key={o.value} value={o.value} className={o.ultra ? 'text-ultra' : undefined}>
        <OptionContent checked={o.value === value} checkSlot>{o.label}</OptionContent>
      </RadioGroup.Item>)}
    </RadioGroup.Root>
  );
}

// What a row's menu holds, for the current model or any other: an embedded family's levels (a Fusion pair as its lead,
// level and sidekick), the native reasoning levels, then Fast and the model's other parameters. `setParam` sets a native
// parameter of this row's model, `onSelect` picks one of its variants
function FamilyParams({ family: f, view, hidden, onSelect, setParam }: {
  family: ModelFamily; view: FamilyView; hidden?: Record<string, string[]>; onSelect: (id: string) => void; setParam: (id: string, value: string) => void;
}) {
  const v = view.variant;
  const embedded = f.variants.length > 1;
  const nativeFast = view.modelConfig.find(isFastControl);
  const fast: FastState | undefined = embedded && f.hasFast
    ? (f.fusion ? fusionFast(f, v, onSelect) : variantFast(f, v, onSelect))
    : nativeFast && nativeFastState(nativeFast, hidden?.[nativeFast.id], value => setParam(nativeFast.id, value));
  const rest = view.modelConfig.filter(control => control !== (fast && !(embedded && f.hasFast) ? nativeFast : undefined));
  const pickEffort = (e: string) => { const hit = findVariant(f, e, v.fast, false); if (hit) onSelect(hit.id); };
  const levels = <>
    {embedded && f.fusion && <FusionParams family={f} variant={v} onSelect={onSelect} />}
    {embedded && !f.fusion && variantScale(f) && <LevelList options={f.efforts.map(e => ({ value: e, label: e || t('composer.standard') }))} value={v.effort} onChange={pickEffort} />}
    {view.reasoning.map(control => {
      const p = presentReasoning(control);
      return p.efforts.length > 1 && <LevelList key={control.id} options={p.efforts.map(o => ({ value: o.id, label: o.name, ultra: isUltraLevel(o.name) }))}
        value={p.off ? '' : (p.value ?? '')} onChange={value => setParam(control.id, value)} />;
    })}
  </>;
  const switches = <>
    {embedded && !f.fusion && thinkingOnly(f) && <SwitchRow label="Thinking" checked={v.effort === 'Thinking'}
      disabled={!findVariant(f, v.effort === 'Thinking' ? '' : 'Thinking', v.fast, false)} onChange={on => pickEffort(on ? 'Thinking' : '')} />}
    {view.reasoning.map(control => {
      const p = presentReasoning(control);
      const turnOn = p.efforts.find(o => o.id === 'high')?.id ?? p.efforts[0]?.id ?? p.onId;
      return p.offId && <SwitchRow key={control.id} label="Thinking" checked={!p.off} onChange={on => { if (on) { if (turnOn) setParam(control.id, turnOn); } else setParam(control.id, p.offId!); }} />;
    })}
    {fast && <SwitchRow label="Fast" checked={fast.on} disabled={fast.disabled} onChange={fast.set} />}
    {rest.map(control => <ModelConfigParams key={control.id} control={control} hidden={hidden?.[control.id]} onChange={value => setParam(control.id, value)} />)}
  </>;
  // The hairline only stands between the two groups
  return <div className="flex flex-col [&>div:empty]:hidden [&>div+div:not(:empty)]:mt-1 [&>div+div:not(:empty)]:border-t [&>div+div:not(:empty)]:border-line [&>div+div:not(:empty)]:pt-1">
    <div className="flex flex-col">{levels}</div>
    <div className="flex flex-col">{switches}</div>
  </div>;
}

// Rows filed under their source like files under a folder, in first-seen order; rows without one lead, unlabelled.
// Nothing to file with fewer than two sources: the list stays flat
interface SourceGroup<T> { value: string; items: T[] }
function bySource<T>(items: T[], sourceOf: (item: T) => string | undefined): SourceGroup<T>[] | undefined {
  const groups = new Map<string, T[]>();
  for (const item of items) {
    const source = sourceOf(item) ?? '';
    groups.set(source, [...(groups.get(source) ?? []), item]);
  }
  if ([...groups.keys()].filter(Boolean).length < 2) return undefined;
  return [...groups].sort(([a], [b]) => Number(!!a) - Number(!!b)).map(([value, items]) => ({ value, items }));
}
// A Command list's rows: flat, or each source's rows under its label
function grouped<T>(groups: SourceGroup<T>[] | undefined, row: (item: T) => ReactNode) {
  if (!groups) return row;
  return (group: SourceGroup<T>) => (
    <Command.Group key={group.value} items={group.items}>
      {group.value && <Command.GroupLabel>{group.value}</Command.GroupLabel>}
      <Command.Collection>{row}</Command.Collection>
    </Command.Group>
  );
}
type SidekickOption = ConfigControl['options'][number];
// A family's source as a category. An id that only tells two equally named models apart is no category: it stays on the row
const familySource = (f: ModelFamily) => (f.source && !f.variants.some(v => v.id === f.source) ? f.source : undefined);

// The model list: the whole panel, or the lead's side of PairSwitch (which sets the panel's height). Every row carries
// the level and Fast its model runs with; a row with parameters opens them in a menu beside it. A lone model keeps
// them under its row instead (`isLone`): a list of one has nothing to switch between
function LeadPanel(props: ModelPanelProps) {
  const { families, cur, curVar, onSelect, close, reasoning = [], modelConfig = [], hiddenConfig, onSetConfig, fold, add } = props;
  const [menu, setMenu] = useState<{ key: string; row: HTMLElement }>();
  const list = useRef<HTMLDivElement>(null);
  const groups = useMemo(() => bySource(families, familySource), [families]);
  const twoLine = families.some(f => f.source && !familySource(f));
  const lone = isLone(props);
  const views = useMemo(() => new Map(families.map(f => [f.key, familyView(f, props)])),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [families, cur, curVar, reasoning, modelConfig, props.model, props.memory, props.shapes]);
  const closeMenu = () => {
    setMenu(undefined);
    // Back where the keys were: the search line, or the list itself
    requestAnimationFrame(() => (list.current?.querySelector<HTMLElement>('input') ?? list.current?.querySelector<HTMLElement>('[role="listbox"]'))?.focus({ preventScroll: true }));
  };
  const opened = menu && families.find(f => f.key === menu.key);
  const openedView = opened && views.get(opened.key);
  return (
    <>
      {/* Always listed, a lone model too: its checked row is what the panel is about */}
      <div ref={list} className="flex min-h-0 flex-col"
        onKeyDown={e => {
          // The right arrow steps into the highlighted row's parameters, unless it is moving the caret through a query
          const input = e.target instanceof HTMLInputElement ? e.target : undefined;
          if (e.key !== 'ArrowRight' || lone || (input && input.selectionStart !== input.value.length)) return;
          const row = list.current?.querySelector<HTMLElement>('[data-highlighted][data-family][data-has-params]');
          if (!row) return;
          e.preventDefault();
          setMenu({ key: row.dataset.family!, row });
        }}>
        <Command.Root<ModelFamily> items={groups ?? families} value={cur ?? null} itemToStringLabel={f => [f.name, f.source].filter(Boolean).join(' ')}
          itemToStringValue={f => f.key} isItemEqualToValue={(a, b) => a.key === b.key}>
          <Command.Input visible={modelSearch(families, fold)} action={add} />
          <Command.Empty />
          <Command.List searchable={modelSearch(families, fold)}>
            {grouped(groups, (f: ModelFamily) => {
              const view = views.get(f.key)!;
              const params = !lone && viewHasParams(f, view);
              return <Command.Item key={f.key} value={f} title={f.description} data-family={f.key} data-has-params={params ? '' : undefined}
                data-params={menu?.key === f.key ? '' : undefined} className={cn('group/row', twoLine && 'min-h-0 py-1.5')}
                onClick={() => { onSelect(view.variant.id); close(); }}>
                <OptionContent icon={<ModelMark family={f.name} brand={f.brand} />} description={familySource(f) ? undefined : f.source}
                  trail={<RowTrail checked={f === cur} label={params ? t('composer.modelParams', { model: f.name }) : undefined}
                    onOpen={params ? row => setMenu(m => (m?.key === f.key ? undefined : { key: f.key, row })) : undefined} />}>
                  <span className="flex min-w-0 items-baseline gap-1"><span className={f.fusion ? 'shrink-0' : 'truncate'}>{f.name}</span>{!lone && <RowParams family={f} view={view} />}</span>
                </OptionContent>
              </Command.Item>;
            })}
          </Command.List>
        </Command.Root>
      </div>
      {opened && openedView && menu.row.isConnected && <ParamsMenu anchor={menu.row} onClose={closeMenu}>
        <FamilyParams family={opened} view={openedView} hidden={hiddenConfig}
          // Another model's parameter is picked together with the model: the engine sets it once the model has landed
          onSelect={onSelect}
          setParam={(id, value) => (openedView.live ? onSetConfig?.(id, value) : onSelect(openedView.variant.id, { ...openedView.picks, [id]: value }))} />
      </ParamsMenu>}
      {lone && <LoneParams {...props} />}
    </>
  );
}

// A lone model's parameters, under its row: a short effort scale fits on one row
function LoneParams({ cur, curVar, onSelect, reasoning = [], modelConfig = [], hiddenConfig, onSetConfig }: ModelPanelProps) {
  const shown = reasoning.filter(reasoningVisible);
  const showParams = !!(cur && curVar && cur.variants.length > 1);
  const nativeFast = modelConfig.find(isFastControl);
  const fast: FastState | undefined = showParams && cur!.hasFast
    ? (cur!.fusion ? fusionFast(cur!, curVar!, onSelect) : variantFast(cur!, curVar!, onSelect))
    : nativeFast && nativeFastState(nativeFast, hiddenConfig?.[nativeFast.id], value => onSetConfig?.(nativeFast.id, value));
  const usedNative = fast && !(showParams && cur!.hasFast) ? nativeFast : undefined;
  const rest = modelConfig.filter(control => control !== usedNative);
  const inline = (showParams ? 1 : 0) + shown.length + rest.length + (fast ? 1 : 0) === 1;
  return (
    <div className="mt-1 flex flex-col gap-0.5 border-t border-line pt-1.5">
      {showParams && (cur!.fusion
        ? <FusionParams family={cur!} variant={curVar!} onSelect={onSelect} />
        : <ModelParams family={cur!} variant={curVar!} onSelect={onSelect} inline={inline} />)}
      {shown.map(control => <ReasoningParams key={control.id} control={control} onChange={value => onSetConfig?.(control.id, value)} inline={inline} />)}
      {fast && <FastRow fast={fast} />}
      {rest.map(control => <ModelConfigParams key={control.id} control={control} hidden={hiddenConfig?.[control.id]} onChange={value => onSetConfig?.(control.id, value)} />)}
    </div>
  );
}

// The pair switch's sidekick side of a Fusion pair: the sidekicks Devin pairs with the lead in use, each with its level as
// the pair names it. A pick is the pair with that sidekick, the lead, its level and Fast staying (findFusionVariant)
function FusionSidekicks({ family: f, variant: v, sidekicks, onSelect }: { family: ModelFamily; variant: ModelVariant; sidekicks: string[]; onSelect: (id: string) => void }) {
  const pick = (sidekick: string) => {
    const hit = findFusionVariant(f, { lead: v.lead ?? '', effort: v.effort, sidekick, fast: v.fast });
    if (hit) onSelect(hit.id);
  };
  const searchable = sidekicks.length >= SEARCH_FROM;
  return <div className="flex min-h-0 flex-1 flex-col">
    <Command.Root<string> items={sidekicks} value={v.sidekick ?? null}>
      <Command.Input visible={searchable} />
      <Command.Empty />
      <Command.List searchable={searchable} fill>
        {(s: string) => {
          const p = parseModelName(s);
          return <Command.Item key={s} value={s} onClick={() => pick(s)}>
            <OptionContent icon={<ModelMark family={p.family} />} checked={s === v.sidekick} checkSlot>
              <span className="flex min-w-0 items-baseline gap-1"><span className="truncate">{p.family}</span>{p.effort && <span className="shrink-0 text-3 text-fg-3">{p.effort}</span>}</span>
            </OptionContent>
          </Command.Item>;
        }}
      </Command.List>
    </Command.Root>
  </div>;
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
    return <FastRow fast={fast} />;
  }
  // An ACP boolean arrives as a synthetic Off/On pair; the panel shows it as the switch it really is
  if (control.type === 'boolean') {
    return <SwitchRow label={control.name} checked={control.value === 'true'} onChange={on => onChange(on ? 'true' : 'false')} />;
  }
  return <SelectRow label={control.name} options={options.map(option => ({ value: option.id, label: option.name }))} value={control.value ?? ''} onChange={onChange} />;
}

// Devin's Fusion pair as its own picker's controls, three rows each opening a menu: Lead, Effort (what that lead offers),
// Sidekick. Every change resolves to a real option through findFusionVariant, so a combination the agent
// doesn't offer lands on the nearest one instead of failing; a sidekick the current lead cannot take is disabled rather than hidden
function FusionParams({ family: f, variant: v, onSelect }: { family: ModelFamily; variant: ModelVariant; onSelect: (id: string) => void }) {
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
      {efforts.length > 1 && <SelectRow label={t('composer.effort')} options={efforts.map(e => ({ value: e, label: e || t('composer.standard') }))} value={v.effort} onChange={e => pick({ effort: e })} />}
      <SelectRow label={t('composer.sidekick')} options={sidekicks.map(s => ({ value: s, label: s, disabled: !offers(s) }))} value={v.sidekick ?? ''} onChange={s => pick({ sidekick: s })} />
    </div>
  );
}

// The sidekick side: none, or any model the sidekick select offers. Every row carries the level and Fast its model would
// run with (the picked one its live selects, the others `shapes`, the view's sidekickShapes) and opens them in the menu
// beside it, like a lead row: a level set on another row picks that model with it. A pick keeps the panel open, since
// the level is set from that row. The side fills the lead's height (`.pair-view`).
// Models from two or more CLIs (the host's select) get a rail of CLI marks down the left edge: the list holds the open
// CLI's models, filed by channel, so a group label always names a channel, never a CLI. A query searches every CLI, each
// one's matches under its mark and, inside it, their channel (picked in lab/sidekick-source-pick.preview.html)
function SidekickPanel({ control, effort, fast: fastControl, shapes, onSetConfig }: {
  control: ConfigControl; effort?: ConfigControl; fast?: ConfigControl; shapes?: ModelShapes; onSetConfig: (id: string, value: string, also?: Record<string, string>) => void;
}) {
  const cur = control.options.find(o => o.id === (control.value ?? NO_SIDEKICK));
  const [menu, setMenu] = useState<{ id: string; row: HTMLElement }>();
  const list = useRef<HTMLDivElement>(null);
  const picked = cur && cur.id !== NO_SIDEKICK ? cur : undefined;
  const clis = useMemo(() => sidekickClis(control.options), [control.options]);
  const railed = clis.length > 1;
  // Every open starts on the picked sidekick's CLI, else the session's own (the select lists it first)
  const [openCli, setOpenCli] = useState(() => picked?.group?.id);
  const open = clis.find(c => c.id === openCli) ?? clis.find(c => c.id === picked?.group?.id) ?? clis[0];
  const [query, setQuery] = useState('');
  const searching = railed && query.trim() !== '';
  const rowOf = (o: SidekickOption): { family: ModelFamily; view: FamilyView } | undefined => {
    if (o.id === NO_SIDEKICK) return;
    const variant: ModelVariant = { id: o.id, name: o.name, effort: '', fast: false, long: false };
    const family: ModelFamily = { key: o.id, name: o.name, variants: [variant], efforts: [], hasFast: false, hasLong: false };
    if (o === picked) return { family, view: { variant, reasoning: effort && reasoningVisible(effort) ? [effort] : [], modelConfig: fastControl ? [fastControl] : [], live: true } };
    const shape = shapes?.[o.id];
    if (!shape) return;
    // What the engine would pick with it is this row's own, so the row spells it out (RowParams reads `picks`)
    const picks = Object.fromEntries(shape.flatMap(c => (c.value === undefined ? [] : [[c.id, c.value]])));
    const split = composerControls(shape);
    return { family, view: { variant, reasoning: split.sidekickEffort.filter(reasoningVisible), modelConfig: split.sidekickFast, live: false, picks } };
  };
  const rows = new Map(control.options.map(o => [o.id, rowOf(o)]));
  const items = useMemo((): SidekickOption[] | SidekickGroup[] => {
    const none = control.options.filter(o => o.id === NO_SIDEKICK);
    if (!railed || !open) return bySource(control.options, sidekickChannel) ?? control.options;
    if (!searching) {
      const shown = [...none, ...open.options];
      return bySource(shown, sidekickChannel) ?? shown;
    }
    return [{ value: '', items: none }, ...clis.flatMap(cli => (bySource(cli.options, sidekickChannel) ?? [{ value: '', items: cli.options }])
      .map(g => ({ value: `${cli.id}/${g.value}`, items: g.items, cli, channel: g.value || undefined })))];
  }, [control.options, railed, open, searching, clis]);
  const closeMenu = () => {
    setMenu(undefined);
    requestAnimationFrame(() => (list.current?.querySelector<HTMLElement>('input') ?? list.current?.querySelector<HTMLElement>('[role="listbox"]'))?.focus({ preventScroll: true }));
  };
  const opened = menu && rows.get(menu.id);
  // Another row's parameter is picked together with its model: the engine sets it once the pick has landed
  const setParam = (id: string, value: string) => {
    if (!menu) return;
    if (menu.id === picked?.id) onSetConfig(id, value);
    else onSetConfig(control.id, menu.id, { [id]: value });
  };
  const openOn = (cli: string | undefined) => {
    setMenu(undefined);
    setQuery('');
    setOpenCli(cli);
  };
  const row = (o: SidekickOption) => {
    const r = rows.get(o.id);
    const params = !!r && viewHasParams(r.family, r.view);
    return <Command.Item key={o.id} value={o} data-family={o.id} data-has-params={params ? '' : undefined} data-params={menu?.id === o.id ? '' : undefined}
      // A pick found by a query lands the list on its CLI
      onClick={() => { onSetConfig(control.id, o.id); if (searching && o.group) openOn(o.group.id); }} className="group/row">
      {o.id === NO_SIDEKICK
        ? <OptionContent icon={<Ban className="text-fg-3" strokeWidth={1.75} />} checked={o === cur} checkSlot={!!cur}>{t('composer.sidekickNone')}</OptionContent>
        : <OptionContent icon={<ModelMark family={o.name} brand={optionBrand(o)} />}
          trail={<RowTrail checked={o === cur} label={params ? t('composer.modelParams', { model: o.name }) : undefined}
            onOpen={params ? el => setMenu(m => (m?.id === o.id ? undefined : { id: o.id, row: el })) : undefined} />}>
          <span className="flex min-w-0 items-baseline gap-1"><span className="truncate">{o.name}</span>{r && <RowParams family={r.family} view={r.view} />}</span>
        </OptionContent>}
    </Command.Item>;
  };
  const searchable = control.options.length >= SEARCH_FROM;
  const body = <Command.List searchable={searchable} fill>
    {searching ? searchGroup(row) : grouped(isGroups(items) ? items : undefined, row)}
  </Command.List>;
  return <>
    <div ref={list} className="flex min-h-0 flex-1 flex-col"
      onKeyDown={e => {
        const input = e.target instanceof HTMLInputElement ? e.target : undefined;
        if (e.key !== 'ArrowRight' || (input && input.selectionStart !== input.value.length)) return;
        const el = list.current?.querySelector<HTMLElement>('[data-highlighted][data-family][data-has-params]');
        if (!el) return;
        e.preventDefault();
        setMenu({ id: el.dataset.family!, row: el });
      }}>
      <Command.Root<SidekickOption> items={items} value={cur ?? null} itemToStringValue={o => o.id} isItemEqualToValue={(a, b) => a.id === b.id}
        inputValue={query} onInputValueChange={setQuery}
        itemToStringLabel={o => (o.id === NO_SIDEKICK ? t('composer.sidekickNone') : [o.name, sidekickChannel(o), o.group?.name].filter(Boolean).join(' '))}>
        <Command.Input visible={searchable} />
        {railed && open
          ? <div className="flex min-h-0 flex-1">
            <CliRail clis={clis} open={searching ? undefined : open.id} onOpen={openOn} />
            <div className="flex min-w-0 flex-1 flex-col"><Command.Empty />{body}</div>
          </div>
          : <><Command.Empty />{body}</>}
      </Command.Root>
    </div>
    {menu && opened && menu.row.isConnected && <ParamsMenu anchor={menu.row} onClose={closeMenu}>
      <FamilyParams family={opened.family} view={opened.view} onSelect={() => {}} setParam={setParam} />
    </ParamsMenu>}
  </>;
}

// A group of the sidekick list: a channel's rows; while a query searches every CLI, also the CLI they run in
interface SidekickGroup extends SourceGroup<SidekickOption> { cli?: SidekickCli; channel?: string }
const isGroups = (items: SidekickOption[] | SidekickGroup[]): items is SidekickGroup[] => !!items[0] && 'items' in items[0];

// The CLIs down the sidekick list's left edge, one mark per row so each lines up with a row beside it; the open one is
// lit, none while a query searches them all. It scrolls on its own, with no bar, however many CLIs there are
function CliRail({ clis, open, onOpen }: { clis: SidekickCli[]; open?: string; onOpen: (cli: string) => void }) {
  const rail = useRef<HTMLDivElement>(null);
  const fade = useScrollFade<HTMLDivElement>();
  const scroller = useCallback((el: HTMLDivElement | null) => { rail.current = el; return fade(el); }, [fade]);
  // The open CLI in view when the panel opens, scrolling the rail alone (scrollIntoView would move the conversation too)
  useLayoutEffect(() => {
    const el = rail.current, on = el?.querySelector<HTMLElement>('[aria-pressed="true"]');
    if (!el || !on) return;
    const er = el.getBoundingClientRect(), or = on.getBoundingClientRect();
    if (or.top < er.top || or.bottom > er.bottom) el.scrollTop += or.top - er.top - (er.height - or.height) / 2;
  }, []);
  // A scrolled rail fades the edge that still hides marks instead of slicing one in half; the rule sits on the outer box
  // so it stays whole while the marks fade
  return <div className="mr-1 flex shrink-0 border-r border-line pr-1">
    <div ref={scroller} role="group" aria-label={t('common.agent')} className="scroll-none scroll-fade flex flex-col overflow-y-auto">
      {clis.map(cli => {
        const on = cli.id === open;
        return <button key={cli.id} type="button" title={cli.name} aria-label={cli.name} aria-pressed={on} onClick={() => onOpen(cli.id)}
          className={cn('flex size-row shrink-0 items-center justify-center rounded-md outline-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-fg-2',
            on ? 'bg-active text-fg-1' : 'text-fg-3 hover:text-fg-1')}>
          <AgentMark id={cli.id} name={cli.name} />
        </button>;
      })}
    </div>
  </div>;
}

// A query's matches across every CLI, one group per CLI and channel under one label: the CLI's mark and name, then the
// channel in the fainter ink. No second, stepped-in label line: a bare channel name between rows read as a broken row
function searchGroup(row: (o: SidekickOption) => ReactNode) {
  return (g: SidekickGroup) => (
    <Command.Group key={g.value} items={g.items}>
      {g.cli && <Command.GroupLabel className="flex items-center gap-2 pl-2">
        <span className="flex size-icon shrink-0 items-center justify-center [&_svg]:size-3"><AgentMark id={g.cli.id} name={g.cli.name} /></span>
        <span className="flex min-w-0 items-baseline gap-1.5"><span className="shrink-0 text-fg-2">{g.cli.name}</span>{g.channel && <span className="truncate">{g.channel}</span>}</span>
      </Command.GroupLabel>}
      <Command.Collection>{row}</Command.Collection>
    </Command.Group>
  );
}

// Params of the current family as a small form under the list: reasoning levels as one segmented scale (every level spelled out, one click),
// Fast as a switch row below it. A lone Standard / Thinking pair is a Thinking switch. Every change applies immediately; 1M variants are
// not offered (a level change keeps the standard context), and a Fast the current level lacks is disabled rather than hidden
function ModelParams({ family: f, variant: v, onSelect, inline }: { family: ModelFamily; variant: ModelVariant; onSelect: (id: string) => void; inline?: boolean }) {
  const pickEffort = (e: string) => { const hit = findVariant(f, e, v.fast, false); if (hit) onSelect(hit.id); };
  return (
    <div className="flex flex-col">
      {variantScale(f) && (
        <EffortField options={f.efforts.map(e => ({ value: e, label: e || t('composer.standard') }))} value={v.effort} onChange={pickEffort} inline={inline && fitsRow(f.efforts.length)} />
      )}
      {thinkingOnly(f) && (
        <SwitchRow label="Thinking" icon={thinkingIcon} checked={v.effort === 'Thinking'} disabled={!findVariant(f, v.effort === 'Thinking' ? '' : 'Thinking', v.fast, false)} onChange={on => pickEffort(on ? 'Thinking' : '')} />
      )}
    </div>
  );
}

// Native thought_level: effort segments (Codex's Ultra tinted as the last one), plus a Thinking switch when the agent can turn it off.
function ReasoningParams({ control, onChange, inline }: { control: ConfigControl; onChange: (value: string) => void; inline?: boolean }) {
  const p = presentReasoning(control);
  const turnOn = p.efforts.find(o => o.id === 'high')?.id ?? p.efforts[0]?.id ?? p.onId;
  return (
    <div className="flex flex-col">
      {p.offId && <SwitchRow label="Thinking" icon={thinkingIcon} checked={!p.off} onChange={on => { if (on) { if (turnOn) onChange(turnOn); } else onChange(p.offId!); }} />}
      {p.efforts.length > 1 && (
        <EffortField options={p.efforts.map(o => ({ value: o.id, label: o.name, ultra: isUltraLevel(o.name) }))} value={p.off ? '' : (p.value ?? '')} onChange={onChange} inline={inline && !p.offId && fitsRow(p.efforts.length)} />
      )}
    </div>
  );
}

// One row holds the label and four segments at the panel's width; a longer scale stands alone, unlabelled
const fitsRow = (levels: number) => levels <= 4;

// Shared ordinal field for embedded variants, Fusion leads and native thought_level: one segmented scale for any level count.
// Under a lone model's row the levels name themselves, so the scale carries no label unless it shares a row with it (`inline`)
function EffortField({ options, value, onChange, inline }: {
  options: { value: string; label: string; ultra?: boolean }[];
  value: string;
  onChange: (value: string) => void;
  inline?: boolean;
}) {
  return <SegmentScale label={t('composer.effort')} options={options} value={value} onChange={onChange} inline={inline} bare={!inline} />;
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
      <DropdownMenu.RadioGroup value={c.value} className="scroll-none flex max-h-pop flex-col overflow-y-auto">
        {c.options.map(o => <DropdownMenu.RadioItem key={o.id} value={o.id} onClick={() => onSelect(o.id)} className={twoLine ? 'min-h-0 py-1.5' : undefined}>{content(o)}</DropdownMenu.RadioItem>)}
      </DropdownMenu.RadioGroup>
    </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
  </DropdownMenu.Root>;
}
