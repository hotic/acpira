import { describe, expect, it } from 'vitest';
import { Brain, ListTodo } from 'lucide-react';
import { todoEntries, toolTodoEntries } from '../src/shared/todoTools';
import type { ToolCallBlock } from '../src/shared/transcript';
import { toolIcon } from '../src/webview/chat/icons';
import { agentTurn } from './fixtures/engine';

const todos = [
  { content: 'Inspect status', status: 'in_progress', priority: 'medium' },
  { content: 'Verify result', status: 'pending', priority: 'medium' },
];
const result = { type: 'Todo', TodosUpdated: { summary_for_prompt: 'List updated', todos } };
const stored: ToolCallBlock = { type: 'tool_call', id: 'todo', kind: 'think', verb: 'Update todos', verbKey: 'verb.todo', status: 'completed', content: { type: 'text', text: JSON.stringify(result) } };

describe('todo tool presentation', () => {
  it('normalizes Grok sparse packets and keeps the standard plan separate', () => {
    const turn = agentTurn('todo-grok-sparse');
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
    expect(toolIcon(agentTurn('todo-meta').blocks[0] as ToolCallBlock)).toBe(ListTodo);
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
    expect(toolTodoEntries(agentTurn('todo-large').blocks[0] as ToolCallBlock)).toEqual(todoEntries(result));
  });
});
