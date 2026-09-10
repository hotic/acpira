import { describe, expect, it } from 'vitest';
import { Brain, ListTodo } from 'lucide-react';
import { applyUpdate, emptyState } from '../src/host/acp/normalize';
import { todoEntries, toolTodoEntries } from '../src/shared/todoTools';
import type { AgentTurn, ToolCallBlock } from '../src/shared/transcript';
import { toolIcon } from '../src/webview/chat/icons';

const todos = [
  { content: 'Inspect status', status: 'in_progress', priority: 'medium' },
  { content: 'Verify result', status: 'pending', priority: 'medium' },
];
const result = { type: 'Todo', TodosUpdated: { summary_for_prompt: 'List updated', todos } };
const stored: ToolCallBlock = { type: 'tool_call', id: 'todo', kind: 'think', verb: 'Update todos', verbKey: 'verb.todo', status: 'completed', content: { type: 'text', text: JSON.stringify(result) } };

describe('todo tool presentation', () => {
  it('normalizes Grok sparse packets and keeps the standard plan separate', () => {
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'todo', title: 'todo_write', rawInput: { todos } });
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'todo', kind: 'think', title: 'Updating plan' });
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'todo', status: 'completed', rawOutput: result });
    applyUpdate(s, { sessionUpdate: 'plan', entries: todos as never });
    const turn = s.turns[0] as AgentTurn;
    const tool = turn.blocks[0] as ToolCallBlock;
    expect(tool.target).toBeUndefined();
    expect(toolIcon(tool)).toBe(ListTodo);
    expect(toolTodoEntries(tool)).toEqual([
      { title: 'Inspect status', status: 'in_progress', priority: 'medium' },
      { title: 'Verify result', status: 'pending', priority: 'medium' },
    ]);
    expect(turn.blocks).toHaveLength(2);
    expect(turn.blocks[1]).toEqual({ type: 'plan', entries: tool.todoEntries, changed: true });
  });

  it('recognizes a sparse Grok update from its tool metadata', () => {
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'todo', title: 'Updating plan', kind: 'think', status: 'completed', _meta: { 'x.ai/tool': { name: 'todo_write' } }, rawOutput: result });
    expect(toolIcon((s.turns[0] as AgentTurn).blocks[0] as ToolCallBlock)).toBe(ListTodo);
  });

  it('renders stored JSON with the same entries and preserves ordinary thinking icons', () => {
    expect(toolTodoEntries(stored)).toEqual(todoEntries(result));
    expect(toolIcon(stored)).toBe(ListTodo);
    expect(toolIcon({ ...stored, verbKey: undefined })).toBe(Brain);
    expect(toolTodoEntries({ ...stored, verbKey: undefined })).toBeUndefined();
  });

  it('parses the confirmed Kimi receipt without exposing the trailing hint', () => {
    const receipt = 'Todo list updated.\nCurrent todo list:\n  [done] Inspect status\n  [pending] Verify result\n\nEnsure that you continue to use the todo list.';
    expect(todoEntries(receipt)).toEqual([{ title: 'Inspect status', status: 'completed' }, { title: 'Verify result', status: 'pending' }]);
  });

  it('does not hide errors or misrepresent partial input, unknown statuses, or malformed output', () => {
    expect(toolTodoEntries({ ...stored, status: 'failed' })).toBeUndefined();
    expect(toolTodoEntries({ ...stored, status: 'pending' })).toBeUndefined();
    expect(todoEntries('{"todos":[')).toBeUndefined();
    expect(todoEntries({ todos: [{ content: 'Unknown', status: 'cancelled' }] })).toBeUndefined();
    expect(todoEntries({ todos: [] })).toEqual([]);
  });

  it('preserves full results before generic output truncation', () => {
    const s = emptyState();
    const large = { ...result, TodosUpdated: { ...result.TodosUpdated, summary_for_prompt: 'x'.repeat(25_000) } };
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'todo', title: 'todo_write', kind: 'think', status: 'completed', rawOutput: large });
    expect(toolTodoEntries((s.turns[0] as AgentTurn).blocks[0] as ToolCallBlock)).toEqual(todoEntries(result));
  });
});
