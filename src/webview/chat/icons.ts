import { Brain, FileText, Globe, ImagePlus, ListTodo, MoveRight, Pencil, Repeat, Search, Terminal, Trash2, Wrench, type LucideIcon } from 'lucide-react';
import type { ToolCallBlock, ToolKind } from '@shared/transcript';
import { isTodoTool } from '@shared/todoTools';
import { isImageGenTool } from '@shared/imageTools';

export function toolIcon(block: ToolCallBlock): LucideIcon {
  return isTodoTool(block) ? ListTodo : isImageGenTool(block) ? ImagePlus : TOOL_ICON[block.kind];
}

export const TOOL_ICON: Record<ToolKind, LucideIcon> = {
  read: FileText,
  edit: Pencil,
  delete: Trash2,
  move: MoveRight,
  search: Search,
  execute: Terminal,
  think: Brain,
  fetch: Globe,
  switch_mode: Repeat,
  other: Wrench,
};
