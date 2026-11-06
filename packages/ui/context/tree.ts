import type { ContextCategory, ContextNode, ContextReport } from '@acpira/shared/context';
import type { MsgKey } from '@acpira/shared/i18n/core';
import { t } from '../i18n';
import { shortPath } from '../settings/paths';
import { categoryLabel } from './categories';

// The report's tree (ContextReport.tsx): the root, the categories, then the agent's nodes (a turn's parts, a server's
// tools) in the order the request carries them. Both the treemap and the tree below it draw this one tree

export interface TNode {
  id: string;
  label: string;
  // What it names beside its label: a call's path or command, a turn's prompt
  target?: string;
  // The label is a name (a tool, a skill, a file), the target code (a call's path or command)
  mono?: boolean;
  monoTarget?: boolean;
  tokens: number;
  cached: number;
  cat: ContextCategory;
  // The agent's node; none for the root and the categories
  node?: ContextNode;
  // A file the leaf names, absolute: an instruction file, or what a read or an edit worked on
  file?: string;
  children?: TNode[];
  // A treemap stub standing for thin sibling tiles: their ids
  more?: string[];
}

export interface TreePaths { home?: string; cwd?: string }

export const ROOT_ID = 'all';

// Verbs of a call by ACP tool kind, as the transcript says them
const VERB: Record<string, MsgKey> = {
  read: 'verb.read', edit: 'verb.edit', delete: 'verb.delete', move: 'verb.move', search: 'verb.search', execute: 'verb.execute',
  think: 'verb.think', fetch: 'verb.fetch', switch_mode: 'verb.switch_mode',
};

const isAbsolute = (p: string) => p.startsWith('/') || /^[A-Za-z]:[\\/]/.test(p);
const join = (dir: string, p: string) => `${dir.replace(/[\\/]+$/, '')}${dir.includes('\\') && !dir.includes('/') ? '\\' : '/'}${p}`;

// A call title naming several paths ends in " +N" (acpira-agent context.rs `call_title`): no single file to open
const SEVERAL = /\s\+\d+$/;

function nodeOf(n: ContextNode, cat: ContextCategory, paths: TreePaths): TNode {
  const base = { id: n.id, tokens: n.tokens, cached: n.cached ?? 0, cat, node: n };
  const short = (p: string) => (paths.home !== undefined && paths.cwd !== undefined ? shortPath(p, { home: paths.home, cwd: paths.cwd }) : p);
  switch (n.kind) {
    case 'base': return { ...base, label: t('context.node.base') };
    case 'environment': return { ...base, label: t('context.node.environment') };
    case 'prompt': return { ...base, label: categoryLabel('system') };
    // A category the agent reports only as a sum: the node is the category itself
    case 'rest': return { ...base, label: categoryLabel(cat) };
    case 'doc': return { ...base, label: n.name ? short(n.name) : categoryLabel('docs'), mono: true, ...(n.name && isAbsolute(n.name) ? { file: n.name } : {}) };
    case 'tool': case 'skill': case 'mcpTool': return { ...base, label: n.name ?? n.id, mono: true };
    case 'skillTool': return { ...base, label: t('context.node.skillTool') };
    case 'summary': return { ...base, label: t('context.node.summary') };
    case 'message': return { ...base, label: t('context.node.message') };
    case 'note': return { ...base, label: t('context.node.note') };
    case 'thinking': return { ...base, label: t('context.node.thinking') };
    case 'reply': return { ...base, label: t('context.node.reply') };
    case 'call': {
      const verb = n.toolKind && VERB[n.toolKind];
      // A tool without a verb of its own (an MCP tool, a goal tool) goes by its name
      const label = verb ? t(verb) : n.name ?? t('verb.other');
      const named = (n.toolKind === 'read' || n.toolKind === 'edit') && n.title && !SEVERAL.test(n.title) ? n.title : undefined;
      const file = named && (isAbsolute(named) ? named : paths.cwd && isAbsolute(paths.cwd) ? join(paths.cwd, named) : undefined);
      return { ...base, label, ...(n.title ? { target: n.title, monoTarget: true } : {}), ...(file ? { file } : {}) };
    }
    case 'server': return { ...base, label: n.name ?? n.id, mono: true };
    case 'turn': return n.index !== undefined
      ? { ...base, label: t('context.node.turn', { n: n.index }), ...(n.title ? { target: n.title } : {}) }
      : { ...base, label: n.title ?? t('context.node.continued') };
  }
}

function build(n: ContextNode, cat: ContextCategory, paths: TreePaths): TNode {
  const out = nodeOf(n, cat, paths);
  if (!n.children) return out;
  const children = n.children.map(c => build(c, cat, paths));
  return { ...out, children, tokens: sum(children, c => c.tokens), cached: sum(children, c => c.cached) };
}

const sum = <T>(xs: T[], f: (x: T) => number) => xs.reduce((s, x) => s + f(x), 0);

export interface Tree {
  root: TNode;
  index: Map<string, TNode>;
  // Every node's ancestors from the root down, itself last
  pathOf: (id: string) => string[];
}

// The tree of a detailed report; categories without nodes (a card-only report) are left out
export function buildTree(r: ContextReport, paths: TreePaths = {}): Tree {
  const cats: TNode[] = r.categories.filter(c => c.nodes?.length).map(c => {
    // A category known only as a sum is one leaf, not a group around a node of the same name
    const only = c.nodes!.length === 1 && c.nodes![0]!.kind === 'rest' ? c.nodes![0]! : undefined;
    if (only) return { id: c.id, label: categoryLabel(c.id), cat: c.id, node: only, tokens: only.tokens, cached: only.cached ?? 0 };
    const children = c.nodes!.map(n => build(n, c.id, paths));
    return { id: c.id, label: categoryLabel(c.id), cat: c.id, children, tokens: sum(children, x => x.tokens), cached: sum(children, x => x.cached) };
  });
  const root: TNode = { id: ROOT_ID, label: '', cat: 'chat', children: cats, tokens: sum(cats, c => c.tokens), cached: sum(cats, c => c.cached) };
  const index = new Map<string, TNode>();
  const parents = new Map<string, string>();
  const walk = (n: TNode) => {
    index.set(n.id, n);
    for (const c of n.children ?? []) { parents.set(c.id, n.id); walk(c); }
  };
  walk(root);
  const pathOf = (id: string) => {
    const out = [id];
    for (let p = parents.get(id); p; p = parents.get(p)) out.unshift(p);
    return out;
  };
  return { root, index, pathOf };
}

export const leafCount = (n: TNode): number => (n.children ? n.children.reduce((s, c) => s + leafCount(c), 0) : 1);

// A share of the used figure, never shown as 0%
export function share(n: number, of: number): string {
  const p = of > 0 ? (n / of) * 100 : 0;
  return p >= 1 ? `${Math.round(p)}%` : '<1%';
}
