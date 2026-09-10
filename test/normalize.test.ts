import { describe, expect, it, vi } from 'vitest';
import { activityOf, applyUpdate, diffLines, emptyState, endTurn, failTurn } from '../src/host/acp/normalize';

describe('diffLines', () => {
  it('LCS line-level diff, keeping only context near changes', () => {
    const old = Array.from({ length: 20 }, (_, i) => `line ${i}`).join('\n');
    const neu = old.replace('line 10', 'LINE 10');
    const lines = diffLines(old, neu);
    expect(lines.filter(l => l.kind === 'del').map(l => l.text)).toEqual(['-line 10']);
    expect(lines.filter(l => l.kind === 'add').map(l => l.text)).toEqual(['+LINE 10']);
    // Leading omission stays (line offset), the trailing one is dropped: nothing follows it
    expect(lines.filter(l => l.kind === 'hunk')).toHaveLength(1);
    expect(lines[0]?.kind).toBe('hunk');
    expect(lines.at(-1)?.kind).toBe('ctx');
    expect(lines.filter(l => l.kind === 'ctx')).toHaveLength(6);
  });

  it('new file is all add', () => {
    expect(diffLines('', 'a\nb').map(l => l.kind)).toEqual(['add', 'add']);
  });
});

describe('applyUpdate', () => {
  it('ignores empty thought deltas while retaining whitespace inside real reasoning', () => {
    const s = emptyState();
    expect(applyUpdate(s, { sessionUpdate: 'agent_thought_chunk', content: { type: 'text', text: '' } })).toBe(false);
    expect(s.turns).toEqual([]);
    for (const text of ['Hello', ' ', 'world', '']) {
      applyUpdate(s, { sessionUpdate: 'agent_thought_chunk', content: { type: 'text', text } });
    }
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'read', title: 'Read', kind: 'read' });
    for (const text of ['', '\n  ']) {
      expect(applyUpdate(s, { sessionUpdate: 'agent_thought_chunk', content: { type: 'text', text } })).toBe(false);
    }
    expect(s.turns[0]).toMatchObject({ blocks: [{ type: 'thought', text: 'Hello world', streaming: false }, { type: 'tool_call' }] });
  });

  it.each(['completed', 'failed'] as const)('handles a Kimi edit diff followed by a %s text result', status => {
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'edit', title: 'Edit', kind: 'edit', status: 'pending' });
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'edit', status: 'in_progress',
      content: [{ type: 'diff', path: 'sample.ts', oldText: 'const n = 1;', newText: 'const n = 2;' }] });
    const turn = s.turns[0];
    if (turn?.role !== 'agent') throw new Error();
    const before = structuredClone(turn.blocks[0]);
    const text = status === 'completed' ? 'Replaced 1 occurrence in sample.ts' : 'File changed before edit';
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'edit', status,
      content: [{ type: 'content', content: { type: 'text', text } }] });
    expect(turn.blocks[0]).toMatchObject(status === 'completed'
      ? { ...before, status }
      : { status, content: { type: 'text', text }, diffStat: undefined });
  });

  it('times observed execution across sparse updates, excluding pending approval and replay', () => {
    const s = emptyState();
    s.turns.push({ role: 'agent', blocks: [], startedAt: 1000 });
    const clock = vi.spyOn(Date, 'now').mockReturnValue(2000);
    try {
      applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'sh', title: 'bash', status: 'pending' });
      expect(s.turns[0]).toMatchObject({ blocks: [expect.not.objectContaining({ startedAt: expect.any(Number) })] });
      clock.mockReturnValue(5000);
      applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'sh', status: 'in_progress' });
      clock.mockReturnValue(8000);
      applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'sh', kind: 'execute', rawInput: { command: 'pnpm test' } });
      expect(s.turns[0]).toMatchObject({ blocks: [{ startedAt: 5000 }] });
      clock.mockReturnValue(14000);
      applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'sh', status: 'completed' });
      clock.mockReturnValue(19000);
      applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'sh', status: 'completed' });
      endTurn(s, 'end_turn');
      expect(s.turns[0]).toMatchObject({ blocks: [{ startedAt: 5000, endedAt: 14000 }] });

      const replay = emptyState();
      applyUpdate(replay, { sessionUpdate: 'tool_call', toolCallId: 'old', title: 'bash', status: 'in_progress' });
      applyUpdate(replay, { sessionUpdate: 'tool_call_update', toolCallId: 'old', status: 'completed' });
      expect(replay.turns[0]).toMatchObject({ blocks: [expect.not.objectContaining({ startedAt: expect.any(Number) })] });
    } finally { clock.mockRestore(); }
  });

  it.each(['cancelled', 'end_turn'] as const)('freezes outstanding tool timers when the turn ends with %s', stop => {
    const s = emptyState();
    s.turns.push({ role: 'agent', blocks: [], startedAt: 1000 });
    const clock = vi.spyOn(Date, 'now').mockReturnValue(2000);
    try {
      applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'sh', kind: 'execute', status: 'in_progress' });
      clock.mockReturnValue(6000);
      endTurn(s, stop);
      expect(s.turns[0]).toMatchObject({ blocks: [{ startedAt: 2000, endedAt: 6000, status: stop === 'cancelled' ? 'cancelled' : 'failed' }] });
    } finally { clock.mockRestore(); }
  });

  it('records the full live turn duration once, without inventing replay timestamps', () => {
    const s = emptyState();
    s.turns.push({ role: 'agent', blocks: [], startedAt: 1000 });
    const clock = vi.spyOn(Date, 'now').mockReturnValue(287000);
    try {
      endTurn(s, 'end_turn');
      expect(s.turns[0]).toMatchObject({ startedAt: 1000, endedAt: 287000 });
      clock.mockReturnValue(300000);
      endTurn(s, 'end_turn');
      expect(s.turns[0]).toMatchObject({ endedAt: 287000 });
      const replay = emptyState();
      replay.turns.push({ role: 'agent', blocks: [] });
      endTurn(replay, 'end_turn');
      expect(replay.turns[0]).not.toHaveProperty('endedAt');
    } finally {
      clock.mockRestore();
    }
  });

  it('switching from a thought block to a text block finalizes it and records the duration', () => {
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'agent_thought_chunk', content: { type: 'text', text: 'a' } });
    applyUpdate(s, { sessionUpdate: 'agent_thought_chunk', content: { type: 'text', text: 'b' } });
    applyUpdate(s, { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'x' } });
    const t = s.turns[0];
    if (t?.role !== 'agent') throw new Error();
    expect(t.blocks[0]).toMatchObject({ type: 'thought', text: 'ab', streaming: false });
    expect((t.blocks[0] as { durationSec?: number }).durationSec).toBeGreaterThanOrEqual(1);
    expect(t.blocks[1]).toMatchObject({ type: 'text', markdown: 'x', streaming: true });
  });

  it('tool_call_update without a matching tool_call inserts one; execute uses rawInput.command as the target', () => {
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'x', kind: 'execute', status: 'in_progress', rawInput: { command: 'ls -la' } });
    const t = s.turns[0];
    if (t?.role !== 'agent') throw new Error();
    expect(t.blocks[0]).toMatchObject({ type: 'tool_call', id: 'x', verb: 'Run', target: 'ls -la', targetMono: true, status: 'in_progress' });
  });

  // Devin 3000.6.14 wire shape: exec past its timeout is parked with cognition.ai/background, then get_output ("Read shell", no kind) blocks on it
  it('Devin background shell: the parked exec is flagged, get_output becomes a wait naming that command, and neither passes for the current action', () => {
    const s = emptyState();
    s.turns.push({ role: 'agent', blocks: [], startedAt: 1 });
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'exec:0', title: 'Ran python3', kind: 'execute', rawInput: { command: 'python3 snap.py save', timeout: 10000 }, _meta: { 'cognition.ai/inferenceToolName': 'exec' } });
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'exec:0', status: 'in_progress', _meta: { 'cognition.ai/inferenceToolName': 'exec' } });
    expect(activityOf(s.turns)?.label).toBe('Run python3 snap.py save');
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'exec:0', status: 'in_progress',
      _meta: { 'cognition.ai/inferenceToolName': 'exec', 'cognition.ai/background': true, 'cognition.ai/backgroundShellId': '0d95e3', 'cognition.ai/backgroundCommand': 'python3 snap.py save' } });
    const t = s.turns[0];
    if (t?.role !== 'agent') throw new Error();
    expect(t.blocks[0]).toMatchObject({ kind: 'execute', status: 'in_progress', background: true, target: 'python3 snap.py save' });
    // The agent has moved on: a streaming thought is the current action, not the parked command
    applyUpdate(s, { sessionUpdate: 'agent_thought_chunk', content: { type: 'text', text: 'checking progress' } });
    expect(activityOf(s.turns)?.label).toBe('Working');
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'get_output:1', title: 'Read shell', rawInput: { shell_id: '0d95e3', timeout: 60000 }, _meta: { 'cognition.ai/inferenceToolName': 'get_output' } });
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'get_output:1', status: 'in_progress' });
    expect(t.blocks[2]).toMatchObject({ kind: 'other', verbKey: 'verb.wait', verb: 'Wait for background command', target: 'python3 snap.py save', targetMono: true, status: 'in_progress' });
    expect(t.blocks[2]).not.toHaveProperty('background');
    expect(activityOf(s.turns)?.label).toBe('Wait for background command python3 snap.py save');
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'get_output:1', status: 'completed', _meta: { 'cognition.ai/inferenceToolName': 'get_output' } });
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'exec:0', status: 'completed', content: [{ type: 'content', content: { type: 'text', text: 'saved 33/33' } }],
      _meta: { 'cognition.ai/inferenceToolName': 'exec', terminal_exit: { terminal_id: '0d95e3', exit_code: 0, signal: null } } });
    expect(t.blocks[0]).toMatchObject({ status: 'completed', content: { type: 'text', text: 'saved 33/33' } });
    expect(t.blocks[2]).toMatchObject({ status: 'completed', target: 'python3 snap.py save' });
  });

  it('a wait on an unknown shell falls back to the shell id, and the title alone identifies the tool without _meta', () => {
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'w', title: 'Read shell', status: 'in_progress', rawInput: { shell_id: 'abc123', timeout: 5000 } });
    const t = s.turns[0];
    if (t?.role !== 'agent') throw new Error();
    expect(t.blocks[0]).toMatchObject({ kind: 'other', verbKey: 'verb.wait', target: 'abc123', targetMono: true });
  });

  it('kill_shell ("Kill shell", no kind) stops the parked command by name', () => {
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'exec_0', title: 'Ran sleep', kind: 'execute', rawInput: { command: 'sleep 120', timeout: 3000 } });
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'exec_0', status: 'in_progress',
      _meta: { 'cognition.ai/background': true, 'cognition.ai/backgroundShellId': '481dbc', 'cognition.ai/backgroundCommand': 'sleep 120' } });
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'kill_shell_3', title: 'Kill shell', rawInput: { shell_id: '481dbc' }, _meta: { 'cognition.ai/inferenceToolName': 'kill_shell' } });
    const t = s.turns[0];
    if (t?.role !== 'agent') throw new Error();
    expect(t.blocks[1]).toMatchObject({ kind: 'other', verbKey: 'verb.kill', verb: 'Stop background command', target: 'sleep 120', targetMono: true });
  });

  it('endTurn: cancelled marks running tools cancelled, the rest failed; the stop reason lands on the turn', () => {
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'a', title: 't', status: 'in_progress' });
    endTurn(s, 'cancelled');
    const t = s.turns[0];
    if (t?.role !== 'agent') throw new Error();
    expect(t.blocks[0]).toMatchObject({ status: 'cancelled' });
    expect(t.stop).toBe('cancelled');
    applyUpdate(s, { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: 'partial' } });
    endTurn(s, 'max_tokens');
    expect(t.stop).toBe('max_tokens');
    expect(t.blocks[1]).toMatchObject({ type: 'text', streaming: false });
  });

  it('failTurn: seals the turn like a cancellation and keeps the error on it', () => {
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'agent_thought_chunk', content: { type: 'text', text: 'hm' } });
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'a', title: 't', status: 'in_progress' });
    failTurn(s, { message: 'Upstream error', code: -32603, kind: 'upstream_error', retryable: true });
    const t = s.turns[0];
    if (t?.role !== 'agent') throw new Error();
    expect(t.stop).toBe('error');
    expect(t.error).toEqual({ message: 'Upstream error', code: -32603, kind: 'upstream_error', retryable: true });
    expect(t.activity).toBeUndefined();
    expect(t.blocks[0]).toMatchObject({ type: 'thought', streaming: false });
    expect(t.blocks[1]).toMatchObject({ status: 'cancelled' });
  });

  it('plan: entries keep their priority, a later plan replaces the whole list within the turn', () => {
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'plan', entries: [{ content: 'a', priority: 'high', status: 'in_progress' }, { content: 'b', priority: 'low', status: 'pending' }] });
    applyUpdate(s, { sessionUpdate: 'plan', entries: [{ content: 'a', priority: 'high', status: 'completed' }, { content: 'b', priority: 'low', status: 'in_progress' }] });
    const t = s.turns[0];
    if (t?.role !== 'agent') throw new Error();
    expect(t.blocks).toHaveLength(1);
    expect(t.blocks[0]).toEqual({ type: 'plan', changed: true, entries: [{ title: 'a', status: 'completed', priority: 'high' }, { title: 'b', status: 'in_progress', priority: 'low' }] });
  });

  it('available_commands_update: keeps the input hint, drops _meta, and a later (even empty) list replaces the previous one', () => {
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'available_commands_update', availableCommands: [
      { name: 'compact', description: 'Compact', _meta: { x: 1 } },
      { name: 'review', description: 'Review', input: { hint: 'files to review', _meta: { y: 2 } } },
      { name: 'plain', description: 'No hint', input: null },
    ] });
    expect(s.commands).toEqual([
      { name: 'compact', description: 'Compact' },
      { name: 'review', description: 'Review', input: { hint: 'files to review' } },
      { name: 'plain', description: 'No hint' },
    ]);
    applyUpdate(s, { sessionUpdate: 'available_commands_update', availableCommands: [{ name: 'review', description: 'Review' }] });
    expect(s.commands.map(c => c.name)).toEqual(['review']);
    applyUpdate(s, { sessionUpdate: 'available_commands_update', availableCommands: [] });
    expect(s.commands).toEqual([]);
  });

  it('configOptions: every select becomes a control, groups flattened, sorted by category, boolean hidden, category=mode promoted to modes', () => {
    const s = emptyState();
    applyUpdate(s, {
      sessionUpdate: 'config_option_update',
      configOptions: [
        { id: 'verbose', name: 'Verbose', type: 'boolean', currentValue: true },
        { id: 'custom', name: 'Style', type: 'select', currentValue: 'x', options: [{ value: 'x', name: 'X' }] },
        { id: 'effort', name: 'Effort', category: 'thought_level', type: 'select', currentValue: 'hi', options: [{ value: 'lo', name: 'Lo' }, { value: 'hi', name: 'Hi' }] },
        { id: 'mode', name: 'Mode', category: 'mode', type: 'select', currentValue: 'plan', options: [{ value: 'agent', name: 'Agent' }, { value: 'plan', name: 'Plan' }] },
        {
          id: 'model', name: 'Model', category: 'model', type: 'select', currentValue: 'b',
          options: [{ group: 'g1', name: 'Group 1', options: [{ value: 'a', name: 'A' }] }, { group: 'g2', name: 'Group 2', options: [{ value: 'b', name: 'B' }] }],
        },
      ],
    });
    expect(s.controls.options.map(o => o.id)).toEqual(['model', 'effort', 'custom']);
    expect(s.controls.options[0]!.options).toEqual([{ id: 'a', name: 'A', description: 'Group 1', group: { id: 'g1', name: 'Group 1' } }, { id: 'b', name: 'B', description: 'Group 2', group: { id: 'g2', name: 'Group 2' } }]);
    expect(s.controls.options[0]!.value).toBe('b');
    expect(s.controls.modes.map(m => m.id)).toEqual(['agent', 'plan']);
    expect(s.controls.modeId).toBe('plan');
    expect(s.controls.modeConfigId).toBe('mode');
  });

  it('configOptions: when modes is non-empty, category=mode is a duplicate (Kimi sends both) and stays out of the control list', () => {
    const s = emptyState();
    s.controls.modes = [{ id: 'default', name: 'Default' }, { id: 'yolo', name: 'YOLO' }];
    s.controls.modeId = 'default';
    applyUpdate(s, {
      sessionUpdate: 'config_option_update',
      configOptions: [
        { id: 'mode', name: 'Mode', category: 'mode', type: 'select', currentValue: 'default', options: [{ value: 'default', name: 'Default' }, { value: 'yolo', name: 'YOLO' }] },
        { id: 'model', name: 'Model', category: 'model', type: 'select', currentValue: 'k3', options: [{ value: 'k3', name: 'K3' }] },
      ],
    });
    expect(s.controls.options.map(o => o.id)).toEqual(['model']);
    expect(s.controls.modes.map(m => m.id)).toEqual(['default', 'yolo']);
    expect(s.controls.modeId).toBe('default');
    expect(s.controls.modeConfigId).toBeUndefined();
  });
});
