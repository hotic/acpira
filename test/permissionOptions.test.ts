import { describe, expect, it } from 'vitest';
import { permissionOption, quickChoices } from '../src/webview/chat/permissionOptions';
import type { PermissionBlock } from '../src/shared/transcript';

type Option = PermissionBlock['options'][number];
const display = (label: string, kind: Option['kind'] = 'allow_always') => permissionOption({ id: 'wire-id', label, kind }, 'zh-CN');

describe('permission option presentation', () => {
  it('preserves command patterns and distinct session, project and global scopes', () => {
    expect(display('Yes, allow `pnpm typecheck` commands (this session)')).toMatchObject({ id: 'wire-id', label: '本会话内允许', detail: '允许 pnpm typecheck 命令' });
    expect(display('Yes, always allow `pnpm test` commands in `other-project`')).toMatchObject({ label: '始终允许 other-project 项目', detail: '允许 pnpm test 命令' });
    expect(display('Yes, always allow `pnpm typecheck` commands in all projects')).toMatchObject({ label: '始终允许所有项目' });
    expect(display('Yes, switch to bypass mode')).toMatchObject({ bypass: true, quick: false });
  });
  it('does not infer scope from kind or discard unfamiliar qualifiers', () => {
    for (const label of ['Allow this one command and exit', 'Yes, allow `pnpm` commands (this session) except deploy', 'Build with Bypass Permissions']) {
      expect(display(label)).toMatchObject({ id: 'wire-id', label, quick: false, detail: undefined });
    }
    expect(display('Allow', 'allow_always')).toMatchObject({ label: 'Allow', quick: false });
    expect(display('Reject and exit Plan mode', 'reject_once')).toMatchObject({ label: 'Reject and exit Plan mode', quick: false });
  });
  it('localizes known quick choices without changing IDs', () => {
    expect(display('Allow', 'allow_once')).toMatchObject({ id: 'wire-id', label: '允许一次', quick: true });
    expect(display('Reject', 'reject_once')).toMatchObject({ id: 'wire-id', label: '拒绝', quick: true });
    expect(permissionOption({ id: 'once', label: 'Allow', kind: 'allow_once' }, 'en').label).toBe('Allow once');
  });
  it('quickChoices picks the first allow_once and reject_once by kind, whatever the labels say', () => {
    // Codex's vocabulary: two reject_once options ("No, continue without…" / "No, and tell Codex…"), no "Allow" label at all
    const opts = [
      { id: 'yes-always', kind: 'allow_always' as const },
      { id: 'yes-proceed', kind: 'allow_once' as const },
      { id: 'yes-once', kind: 'allow_once' as const },
      { id: 'no-skip', kind: 'reject_once' as const },
      { id: 'no-differently', kind: 'reject_once' as const },
    ];
    expect(quickChoices(opts)).toEqual({ allow: opts[1], reject: opts[3] });
    expect(quickChoices([{ id: 'a', kind: 'allow_always' as const }])).toEqual({ allow: undefined, reject: undefined });
  });
});
