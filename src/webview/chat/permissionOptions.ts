import type { PermissionBlock, PermissionKind } from '@shared/transcript';
import { translate, type Locale } from '@shared/i18n';

type Option = PermissionBlock['options'][number];

// Quick buttons are positional: the first allow_once in wire order is the allow action, the first reject_once the
// reject — regardless of what the agent labeled them. `quick` below only marks a label that was shortened
export function quickChoices<T extends { kind: PermissionKind }>(options: T[]): { allow?: T; reject?: T } {
  return { allow: options.find(o => o.kind === 'allow_once'), reject: options.find(o => o.kind === 'reject_once') };
}

// ACP kinds do not describe command patterns or scope. Only shorten complete,
// known CLI labels; unfamiliar choices retain their original text and IDs.
export function permissionOption(option: Option, locale: Locale) {
  const t = (key: Parameters<typeof translate>[1], params?: Parameters<typeof translate>[2]) => translate(locale, key, params);
  // `detail` is the option's own `_meta.permission.description`; a scoped pattern may replace it with the command list
  const base = { ...option, detail: option.detail as string | undefined, bypass: false, quick: false };
  if (option.kind === 'allow_once' && /^(Allow|Allow once|Yes, allow once)$/i.test(option.label)) {
    return { ...base, label: t('permission.once'), quick: true };
  }
  if (option.kind === 'reject_once' && /^(Reject|Reject once|Deny|No, reject)$/i.test(option.label)) {
    return { ...base, label: t('permission.reject'), quick: true };
  }
  if (option.kind !== 'allow_always') return base;
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
