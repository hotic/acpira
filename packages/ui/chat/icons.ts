import { AppWindow, BookOpen, Brain, Clock, FileText, FileVideo, Globe, History, Image, ImagePlus, ListTodo, MessagesSquare, MoveRight, Pencil, Repeat, Search, Terminal, Trash2, Wrench, type LucideIcon } from 'lucide-react';
import type { ToolCallBlock, ToolKind } from '@acpira/shared/transcript';
import { isTodoTool } from '@acpira/shared/todoTools';
import { isImageGenTool, isShowImageTool, isShowVideoTool, isShowWidgetTool } from '@acpira/shared/imageTools';
import { isSessionTool } from '@acpira/shared/sessionLinks';
import { isSkillTool } from './folding';
import { computerIcon, isComputerTool } from './computerTools';

export function toolIcon(block: ToolCallBlock): LucideIcon {
  if (isSkillTool(block)) return BookOpen;
  if (block.verbKey === 'verb.scheduleCron') return Repeat;
  if (block.verbKey === 'verb.schedule' || block.verbKey === 'verb.wakeup' || block.verbKey === 'verb.wakeupStop') return Clock;
  if (isSessionTool(block)) return block.verbKey === 'verb.readSession' ? MessagesSquare : History;
  if (isComputerTool(block)) return computerIcon(block);
  return isTodoTool(block) ? ListTodo : isImageGenTool(block) ? ImagePlus : isShowImageTool(block) ? Image : isShowVideoTool(block) ? FileVideo : isShowWidgetTool(block) ? AppWindow : TOOL_ICON[block.kind];
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
