import { describe, expect, it } from 'vitest';
import { ambiguousChoices, permissionOption, permissionTitle, planApprovalTitle, quickChoices } from '../src/webview/chat/permissionOptions';
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
  it('localizes Claude plan decisions while preserving their distinct approval policies', () => {
    expect(display('Yes, manually approve edits', 'allow_once')).toMatchObject({ id: 'wire-id', label: '开始实施' });
    expect(display('No, keep planning', 'reject_once').label).toBe('继续规划');
    expect(display('Yes, and use auto mode').label).toBe('开始实施，使用自动模式');
    expect(display('Yes, auto-accept edits').label).toBe('开始实施，自动接受编辑');
    expect(display('Yes, and bypass permissions')).toMatchObject({ label: '开始实施，绕过权限审批', bypass: true });
    expect(display('Yes, clear context (73% used) and use auto mode').label).toBe('清空上下文（已用 73%）并使用自动模式');
    expect(display('Yes, clear context and bypass permissions')).toMatchObject({ label: '清空上下文并绕过权限审批', bypass: true });
    expect(display('Yes, clear context (12.5% used) and auto-accept edits').label).toBe('清空上下文（已用 12.5%）并自动接受编辑');
    expect(display('Yes, enter plan mode', 'allow_once').label).toBe('进入计划模式');
    expect(display('No, start implementing now', 'reject_once').label).toBe('立即开始实施');
  });
  it('localizes Codex plan review without treating it as a generic approval', () => {
    expect(display('Yes, implement this plan', 'allow_once')).toMatchObject({ id: 'wire-id', label: '执行这个计划' });
    expect(display('No, and tell Codex what to do differently', 'reject_once').label).toBe('拒绝，并向 Codex 反馈修改意见');
  });
  it('localizes linked plan extras without collapsing Kimi reject-and-exit into revise', () => {
    const show = (label: string, kind: Option['kind']) => permissionOption({ id: 'wire-id', label, kind }, 'zh-CN', true);
    expect(show('Build', 'allow_once').label).toBe('执行');
    expect(show('Revise', 'reject_once').label).toBe('修改计划');
    expect(show('Build with Bypass Permissions', 'allow_always')).toMatchObject({ id: 'wire-id', label: '执行并绕过权限审批', bypass: true });
    expect(show('Reject and exit Plan mode', 'reject_once').label).toBe('拒绝并退出计划模式');
  });
  it('preserves unknown qualifiers, mismatched kinds, option details and English presentation', () => {
    expect(display('Yes, manually approve edits', 'allow_always').label).toBe('Yes, manually approve edits');
    expect(display('Yes, clear context (73% used) and use auto mode except deploy').label).toBe('Yes, clear context (73% used) and use auto mode except deploy');
    expect(permissionOption({ id: 'auto', label: 'Yes, and use auto mode', kind: 'allow_always', detail: 'Custom policy' }, 'zh-CN')).toMatchObject({ id: 'auto', detail: 'Custom policy' });
    expect(permissionOption({ id: 'clear', label: 'Yes, clear context (73% used) and use auto mode', kind: 'allow_always' }, 'en').label).toBe('Yes, clear context (73% used) and use auto mode');
  });
  it('localizes adapter titles and persisted host wrappers, leaving custom titles verbatim', () => {
    expect(planApprovalTitle('Approve Plan', 'zh-CN')).toBe('批准计划');
    expect(permissionTitle('Ready to code?', 'zh-CN')).toBe('准备开始实施？');
    expect(permissionTitle('Implement this plan?', 'zh-CN')).toBe('要执行这个计划吗？');
    expect(permissionTitle('需要批准：切换模式 Approve Plan', 'zh-CN')).toBe('需要批准：批准计划');
    expect(permissionTitle('Approval needed: Switch mode Approve Plan', 'zh-CN')).toBe('需要批准：批准计划');
    expect(permissionTitle('承認が必要：モード切り替え Approve Plan', 'en')).toBe('Approval needed: Approve Plan');
    expect(permissionTitle('Требуется подтверждение: Approve Plan', 'de')).toBe('Genehmigung erforderlich: Plan genehmigen');
    expect(permissionTitle('需要批准：切换模式 Approve Plan', 'en')).toBe('Approval needed: Approve Plan');
    expect(permissionTitle('Approval needed: Deploy production plan', 'zh-CN')).toBe('Approval needed: Deploy production plan');
    expect(planApprovalTitle('toString', 'zh-CN')).toBe('toString');
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
  it('flags two allow_once options as ambiguous, so neither becomes the emphasized quick button', () => {
    // antigravity-acp 1.2.1 ask_question: every answer is allow_once, only deny / dont_trust / block are reject_once
    const answers = [{ id: 'blue', kind: 'allow_once' as const }, { id: 'green', kind: 'allow_once' as const }];
    expect(ambiguousChoices(answers)).toBe(true);
    expect(ambiguousChoices([...answers.slice(0, 1), { id: 'deny', kind: 'reject_once' as const }])).toBe(false);
    expect(ambiguousChoices([{ id: 'allow_always', kind: 'allow_always' as const }, { id: 'allow', kind: 'allow_once' as const }, { id: 'deny', kind: 'reject_once' as const }])).toBe(false);
    // Codex's permission profile: two per-turn grants under a session grant stay a ladder
    expect(ambiguousChoices([...answers, { id: 'session', kind: 'allow_always' as const }])).toBe(false);
  });
});
