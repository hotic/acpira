import { describe, expect, it } from 'vitest';
import type { SessionControls, SessionSummary, Turn } from '@acpira/shared/transcript';
import { cycleComposerMode } from '../chat/Composer';
import { pendingQuickPermission } from '../chat/permissionOptions';
import { nextAttentionSession, orderedSessions, stepSession } from '../chat/sessionNav';
import { panel, panelState } from '../panel/store';

const summary = (id: string, overrides: Partial<SessionSummary> = {}): SessionSummary => ({
  id,
  title: id,
  agent: 'codex',
  cwd: '/proj',
  updatedAt: '2026-04-17T10:00:00.000Z',
  ...overrides,
});

describe('P0 & P1 shortcut models', () => {
  it('cycleComposerMode cycles collaboration, ACP modes, then permission options', () => {
    const picked: Array<[string, string]> = [];
    const modes: string[] = [];
    const controls: SessionControls = {
      modeId: 'agent',
      modes: [{ id: 'agent', name: 'Agent' }, { id: 'plan', name: 'Plan' }],
      options: [],
    };
    expect(cycleComposerMode(controls, id => modes.push(id), (k, v) => picked.push([k, v]))).toBe(true);
    expect(modes).toEqual(['plan']);

    const withCollab: SessionControls = {
      ...controls,
      options: [{
        id: 'mode',
        name: 'Mode',
        category: 'collaboration_mode',
        value: 'code',
        options: [{ id: 'code', name: 'Code' }, { id: 'plan', name: 'Plan' }, { id: 'ask', name: 'Ask' }],
      }],
    };
    expect(cycleComposerMode(withCollab, id => modes.push(id), (k, v) => picked.push([k, v]))).toBe(true);
    expect(picked).toEqual([['mode', 'plan']]);
  });

  it('pendingQuickPermission picks unambiguous pending approvals and skips plan cards or multi-choice asks', () => {
    const turns: Turn[] = [{
      role: 'agent',
      blocks: [
        {
          type: 'permission',
          id: 'plan-linked',
          title: 'Build plan',
          planId: 'plan-doc-1',
          options: [
            { id: 'build', label: 'Build', kind: 'allow_once' },
            { id: 'revise', label: 'Revise', kind: 'reject_once' },
          ],
        },
        {
          type: 'permission',
          id: 'plan-1',
          title: 'Ready to code?',
          options: [
            { id: 'yes', label: 'Yes', kind: 'allow_once' },
            { id: 'no', label: 'No', kind: 'reject_once' },
          ],
        },
        {
          type: 'permission',
          id: 'perm-1',
          title: 'Run command',
          options: [
            { id: 'allow-session', label: 'Allow session', kind: 'allow_always' },
            { id: 'allow-once', label: 'Allow', kind: 'allow_once' },
            { id: 'reject-once', label: 'Reject', kind: 'reject_once' },
          ],
        },
      ],
    }];
    expect(pendingQuickPermission(turns)).toEqual({
      blockId: 'perm-1',
      primaryId: 'allow-once',
      rejectId: 'reject-once',
    });

    // Ambiguous allow_once answers (question disguised as permission) must not quick-approve on Enter
    const ambiguous: Turn[] = [{
      role: 'agent',
      blocks: [{
        type: 'permission',
        id: 'amb-1',
        title: 'Pick color',
        options: [
          { id: 'blue', label: 'Blue', kind: 'allow_once' },
          { id: 'green', label: 'Green', kind: 'allow_once' },
          { id: 'cancel', label: 'Cancel', kind: 'reject_once' },
        ],
      }],
    }];
    expect(pendingQuickPermission(ambiguous)).toBeUndefined();
  });

  it('steps sessions with wrap-around and jumps to waiting sessions before unread/error', () => {
    const list = orderedSessions({
      sessions: [
        summary('s1', { updatedAt: '2026-04-17T10:03:00.000Z' }),
        summary('s2', { updatedAt: '2026-04-17T10:02:00.000Z', state: 'unread', unread: true }),
        summary('s3', { updatedAt: '2026-04-17T10:01:00.000Z', state: 'waiting' }),
        summary('archived', { updatedAt: '2026-04-17T10:04:00.000Z', archived: true }),
      ],
      workspace: '/proj',
      scope: 'workspace',
    });
    expect(list.map(s => s.id)).toEqual(['s1', 's2', 's3']);
    expect(stepSession(list, 's1', 1)).toBe('s2');
    expect(stepSession(list, 's1', -1)).toBe('s3');
    // Waiting session s3 wins over unread s2, then from s3 falls back to unread s2
    expect(nextAttentionSession(list, 's1')).toBe('s3');
    expect(nextAttentionSession(list, 's3')).toBe('s2');
  });

  it('closes active panel tabs before letting ⌘W close the window', () => {
    panel.setContext('s-close', '/proj-close');
    panel.toggle(false);
    expect(panel.closeActive(false)).toBe(false);
    panel.openReview('/proj-close');
    expect(panelState().open).toBe(true);
    const bId = panel.toggleBrowser(false);
    expect(bId).toBeTruthy();
    expect(panelState().tab).toBe(bId);
    // First closeActive closes the browser tab, keeping the Files review tab open
    expect(panel.closeActive(false)).toBe(true);
    expect(panelState().open).toBe(true);
    // Second closeActive closes the last tab and folds the panel
    expect(panel.closeActive(false)).toBe(true);
    expect(panelState().open).toBe(false);
    // Third closeActive reports false so the shell closes the window
    expect(panel.closeActive(false)).toBe(false);
  });
});
