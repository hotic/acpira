import { AppWindow, BookOpen, Brain, FileText, Globe, History, Image, ImagePlus, ListTodo, MessagesSquare, MoveRight, Pencil, Repeat, Search, Terminal, Trash2, Wrench, type LucideIcon } from 'lucide-react';
import type { ToolCallBlock, ToolKind } from '@acpira/shared/transcript';
import { isTodoTool } from '@acpira/shared/todoTools';
import { isImageGenTool, isShowImageTool, isShowWidgetTool } from '@acpira/shared/imageTools';
import { isSessionTool } from '@acpira/shared/sessionLinks';
import { isSkillTool } from './folding';

export function toolIcon(block: ToolCallBlock): LucideIcon {
  if (isSkillTool(block)) return BookOpen;
  if (isSessionTool(block)) return block.verbKey === 'verb.readSession' ? MessagesSquare : History;
  return isTodoTool(block) ? ListTodo : isImageGenTool(block) ? ImagePlus : isShowImageTool(block) ? Image : isShowWidgetTool(block) ? AppWindow : TOOL_ICON[block.kind];
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
