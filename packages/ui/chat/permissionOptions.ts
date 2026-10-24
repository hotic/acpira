import type { PermissionBlock, PermissionKind } from '@shared/transcript';
import { DICTS, en, LOCALES, translate, type Locale, type MsgKey } from '@shared/i18n';

type Option = PermissionBlock['options'][number];

// Match complete adapter headings only; custom mode names remain agent-owned text.
const PLAN_TITLES: Record<string, MsgKey> = {
  'Approve Plan': 'permission.plan.approve',
  'Ready to code?': 'permission.plan.ready',
  'Implement this plan?': 'permission.plan.implementTitle',
  'Enter plan mode': 'permission.plan.enterTitle',
  EnterPlanMode: 'permission.plan.enterTitle',
  'Exit plan mode': 'permission.plan.exitTitle',
  ExitPlanMode: 'permission.plan.exitTitle',
  exit_plan_mode: 'permission.plan.exitTitle',
};

export function planApprovalTitle(title: string, locale: Locale): string {
  const key = Object.hasOwn(PLAN_TITLES, title) ? PLAN_TITLES[title] : undefined;
  return key ? translate(locale, key) : title;
}

const escape = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

// The host's `host.needApprovalFor` wrapper (optionally around `verb.switch_mode`) in every shipped locale,
// since a persisted card keeps the wording of the locale it was recorded under
const WRAPPED_TITLE = (() => {
  const prefixes = new Set(LOCALES.map(l => {
    const wrapper = (DICTS[l]['host.needApprovalFor'] ?? en['host.needApprovalFor']).split('{what}')[0]!;
    return `${escape(wrapper)}(?:${escape(DICTS[l]['verb.switch_mode'] ?? en['verb.switch_mode'])} )?`;
  }));
  return new RegExp(`^(?:${[...prefixes].join('|')})(.+)$`);
})();

export function permissionTitle(title: string, locale: Locale): string {
  const direct = planApprovalTitle(title, locale);
  if (direct !== title || Object.hasOwn(PLAN_TITLES, title)) return direct;
  // Older cards already contain the host's localized wrapper. Rebuild that
  // wrapper at render time so persisted cards follow the current UI language.
  const heading = WRAPPED_TITLE.exec(title)?.[1];
  return heading && Object.hasOwn(PLAN_TITLES, heading)
    ? translate(locale, 'host.needApprovalFor', { what: planApprovalTitle(heading, locale) }) : title;
}

const PLAN_OPTIONS: { kind: PermissionKind; labels: string[]; key: MsgKey; bypass?: boolean }[] = [
  { kind: 'allow_once', labels: ['Yes, manually approve edits', 'Yes, and manually approve edits'], key: 'permission.plan.manual' },
  { kind: 'reject_once', labels: ['No, keep planning'], key: 'permission.plan.continue' },
  { kind: 'allow_always', labels: ['Yes, and use auto mode', 'Yes, and use "auto" mode'], key: 'permission.plan.auto' },
  { kind: 'allow_always', labels: ['Yes, auto-accept edits', 'Yes, and auto-accept edits'], key: 'permission.plan.acceptEdits' },
  { kind: 'allow_always', labels: ['Yes, and bypass permissions'], key: 'permission.plan.bypass', bypass: true },
  { kind: 'allow_once', labels: ['Yes, implement this plan'], key: 'permission.plan.implement' },
  // Codex also uses this rejection on ordinary approvals; retain its exact meaning.
  { kind: 'reject_once', labels: ['No, and tell Codex what to do differently'], key: 'permission.feedbackCodex' },
  { kind: 'allow_once', labels: ['Yes, enter plan mode'], key: 'permission.plan.enter' },
  { kind: 'reject_once', labels: ['No, start implementing now'], key: 'permission.plan.start' },
];

// Quick buttons are positional: the first allow_once in wire order is the allow action, the first reject_once the
// reject — regardless of what the agent labeled them. `quick` below only marks a label that was shortened
export function quickChoices<T extends { kind: PermissionKind }>(options: T[]): { allow?: T; reject?: T } {
  return { allow: options.find(o => o.kind === 'allow_once'), reject: options.find(o => o.kind === 'reject_once') };
}

// Several allow_once options and no allow_always make a list of answers, not an approval ladder (Antigravity's
// ask_question sends every answer as allow_once): the card then lists every option as an equal button in wire order,
// with nothing emphasized. Codex's two per-turn grants come with an allow_always and keep the usual layout. Mirrors the
// engine's `ambiguous_allow`
export function ambiguousChoices<T extends { kind: PermissionKind }>(options: T[]): boolean {
  return !options.some(o => o.kind === 'allow_always') && options.filter(o => o.kind === 'allow_once').length > 1;
}

// ACP kinds do not describe command patterns or scope. Only shorten complete,
// known CLI labels; unfamiliar choices retain their original text and IDs.
export function permissionOption(option: Option, locale: Locale, plan = false) {
  const t = (key: Parameters<typeof translate>[1], params?: Parameters<typeof translate>[2]) => translate(locale, key, params);
  // `detail` is the option's own `_meta.permission.description`; a scoped pattern may replace it with the command list
  const base = { ...option, detail: option.detail as string | undefined, bypass: false, quick: false };
  const known = PLAN_OPTIONS.find(p => p.kind === option.kind && p.labels.includes(option.label));
  if (known) return { ...base, label: t(known.key), bypass: known.bypass ?? false };
  // Build/Revise are generic words outside a linked plan. Kimi's reject-and-exit
  // stays distinct from continuing to revise, including in the extra-options menu.
  if (plan) {
    if (option.kind === 'allow_once' && option.label === 'Build') return { ...base, label: t('plan.build') };
    if (option.kind === 'reject_once' && option.label === 'Revise') return { ...base, label: t('plan.revise') };
    if (option.kind === 'allow_always' && option.label === 'Build with Bypass Permissions') {
      return { ...base, label: t('permission.plan.buildBypass'), bypass: true };
    }
    if (option.kind === 'reject_once' && option.label === 'Reject and exit Plan mode') {
      return { ...base, label: t('permission.plan.rejectExit') };
    }
  }
  if (option.kind === 'allow_once' && /^(Allow|Allow once|Yes, allow once)$/i.test(option.label)) {
    return { ...base, label: t('permission.once'), quick: true };
  }
  if (option.kind === 'reject_once' && /^(Reject|Reject once|Deny|No, reject)$/i.test(option.label)) {
    return { ...base, label: t('permission.reject'), quick: true };
  }
  if (option.kind !== 'allow_always') return base;
  const clear = /^Yes, clear context(?: \((\d+(?:\.\d+)?)% used\))? and (use auto mode|bypass permissions|auto-accept edits)$/.exec(option.label);
  if (clear) {
    const key = clear[2] === 'use auto mode' ? 'permission.plan.clearAuto'
      : clear[2] === 'bypass permissions' ? 'permission.plan.clearBypass' : 'permission.plan.clearAcceptEdits';
    return { ...base, label: t(key, { usage: clear[1] ? t('permission.plan.usage', { percent: clear[1] }) : '' }), bypass: clear[2] === 'bypass permissions' };
  }
  if (option.label === 'Yes, switch to bypass mode') {
    return { ...base, label: t('permission.bypass'), bypass: true };
  }
  const session = /^Yes, allow `([^`]+)` commands \(this session\)$/.exec(option.label);
  const project = /^Yes, always allow `([^`]+)` commands in `([^`]+)`$/.exec(option.label);
  const all = /^Yes, always allow `([^`]+)` commands in all projects$/.exec(option.label);
  const command = session?.[1] ?? project?.[1] ?? all?.[1];
  if (!command) return base;
  return {
    ...base,
    label: session ? t('permission.session') : project ? t('permission.project', { project: project[2]! }) : t('permission.allProjects'),
    detail: t('permission.commands', { command }),
  };
}
