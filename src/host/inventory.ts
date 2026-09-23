import { readdir, readFile, stat } from 'node:fs/promises';
import { isAbsolute, join } from 'node:path';
import type { AgentId } from '@shared/transcript';
import type { AdapterInfo, AgentHealth, AgentInventory, AgentRuntimeInfo, InventoryFile, InventoryMcp, InventoryScope, InventorySkill, McpTransport } from '@shared/inventory';
import type { AgentExt, McpSource, RuleSource } from './agentExt';

// Read-only scan of an agent's extension points (see agentExt.ts). Pure node: no vscode import, so it runs under vitest against temp directories

export interface ScanEnv {
  home: string;
  // Workspace root (the session cwd)
  cwd: string;
  platform?: NodeJS.Platform;
  env?: NodeJS.ProcessEnv;
}

export interface ScanInput {
  agent: AgentId;
  ext?: AgentExt;
  binary: string | null;
  runtime?: AgentRuntimeInfo;
  adapter?: AdapterInfo;
  health?: AgentHealth;
}

export async function scanInventory(input: ScanInput, env: ScanEnv): Promise<AgentInventory> {
  const { agent, ext, binary, runtime, adapter, health } = input;
  const base: AgentInventory = { agent, binary, runtime, adapter, health, steer: ext?.steer ?? false, config: [], mcp: [], skills: [], rules: [], scannedAt: new Date().toISOString() };
  if (!ext) return base;
  const [config, mcp, skills, rules] = await Promise.all([
    Promise.all(ext.config.map(p => fileInfo(p, env))),
    Promise.all(ext.mcp.map(s => readMcp(s, env))),
    Promise.all(ext.skills.map(d => readSkills(d, env))),
    Promise.all(ext.rules.map(r => readRules(r, env))),
  ]);
  // Devin lists ~/.agents/skills and .agents/skills for Kimi too — one skill directory can show up once per template, but never twice for the same path
  return { ...base, config, mcp: mcp.flat(), skills: dedupe(skills.flat(), s => s.path), rules: dedupe(rules.flat(), r => r.path) };
}

// `~/x` → home; `$CONFIG/x` → XDG config home (APPDATA on Windows); relative → workspace; absolute stays
export function expandPath(template: string, env: ScanEnv): string {
  if (template.startsWith('~/')) return join(env.home, template.slice(2));
  if (template.startsWith('$CONFIG/')) return join(configHome(env), template.slice('$CONFIG/'.length));
  return isAbsolute(template) ? template : join(env.cwd, template);
}

export function scopeOf(template: string): InventoryScope {
  return template.startsWith('~/') || template.startsWith('$CONFIG/') || isAbsolute(template) ? 'user' : 'project';
}

function configHome(env: ScanEnv): string {
  const platform = env.platform ?? process.platform;
  const vars = env.env ?? process.env;
  if (platform === 'win32' && vars.APPDATA) return vars.APPDATA;
  return vars.XDG_CONFIG_HOME || join(env.home, '.config');
}

async function fileInfo(template: string, env: ScanEnv): Promise<InventoryFile> {
  const path = expandPath(template, env);
  try {
    const s = await stat(path);
    return { path, scope: scopeOf(template), exists: s.isFile(), size: s.isFile() ? s.size : undefined };
  } catch {
    return { path, scope: scopeOf(template), exists: false };
  }
}

function dedupe<T>(items: T[], key: (t: T) => string): T[] {
  const seen = new Set<string>();
  return items.filter(it => { const k = key(it); if (seen.has(k)) return false; seen.add(k); return true; });
}

// MCP declarations

async function readMcp(source: McpSource, env: ScanEnv): Promise<InventoryMcp[]> {
  const path = expandPath(source.path, env);
  let text: string;
  try { text = await readFile(path, 'utf8'); } catch { return []; }
  const scope = scopeOf(source.path);
  const entries = source.format === 'toml' ? parseTomlMcp(text) : source.format === 'opencode' ? parseOpencodeMcp(text) : parseJsonMcp(text);
  return entries.map(e => ({ ...e, source: path, scope }));
}

type McpEntry = Omit<InventoryMcp, 'source' | 'scope'>;

// { "mcpServers": { name: { command, args, env } | { url } | { transport: "sse", url } } } — Devin / Kimi / Claude-compatible .mcp.json / ~/.claude.json
export function parseJsonMcp(text: string): McpEntry[] {
  const data = parseJsonLoose(text);
  const servers = isRecord(data) && isRecord(data.mcpServers) ? data.mcpServers : undefined;
  if (!servers) return [];
  return Object.entries(servers).flatMap(([name, v]) => {
    if (!isRecord(v)) return [];
    const command = str(v.command);
    const url = str(v.url) ?? str(v.serverUrl) ?? str(v.httpUrl);
    const args = Array.isArray(v.args) ? v.args.filter((a): a is string => typeof a === 'string') : [];
    const kind = str(v.type) ?? str(v.transport);
    return [{ name, transport: transportOf(kind, command, url), target: command ? [command, ...args].join(' ') : url ?? '', enabled: v.enabled !== false && v.disabled !== true }];
  });
}

// [mcp_servers.name] tables with command / args / url / type / enabled — Grok's config.toml. Enough of TOML for these tables, nothing more
export function parseTomlMcp(text: string): McpEntry[] {
  const out: McpEntry[] = [];
  let cur: { name: string; command?: string; args: string[]; url?: string; type?: string; enabled: boolean } | undefined;
  const flush = () => { if (cur) out.push({ name: cur.name, transport: transportOf(cur.type, cur.command, cur.url), target: cur.command ? [cur.command, ...cur.args].join(' ') : cur.url ?? '', enabled: cur.enabled }); cur = undefined; };
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line || line.startsWith('#')) continue;
    if (line.startsWith('[')) {
      flush();
      const m = /^\[\s*mcp_servers\.(?:"([^"]+)"|([^\]\s.]+))\s*\]$/.exec(line);
      if (m) cur = { name: (m[1] ?? m[2])!, args: [], enabled: true };
      continue;
    }
    if (!cur) continue;
    const kv = /^([A-Za-z_][\w-]*)\s*=\s*(.+?)\s*(?:#.*)?$/.exec(line);
    if (!kv) continue;
    const [, key, value] = kv;
    switch (key) {
      case 'command': cur.command = tomlString(value!); break;
      case 'url': cur.url = tomlString(value!); break;
      case 'type': case 'transport': cur.type = tomlString(value!); break;
      case 'enabled': cur.enabled = value!.trim() !== 'false'; break;
      case 'args': cur.args = [...value!.matchAll(/"((?:[^"\\]|\\.)*)"|'([^']*)'/g)].map(m => (m[1] ?? m[2] ?? '').replace(/\\"/g, '"')); break;
      default: break;
    }
  }
  flush();
  return out;
}

// { "mcp": { name: { type: "local", command: [...] } | { type: "remote", url } } } — OpenCode's opencode.json(c).
// `local` is a stdio child process; `remote` is streamable HTTP (our `http` bucket). A missing type is inferred from command / url
export function parseOpencodeMcp(text: string): McpEntry[] {
  const data = parseJsonLoose(text);
  const servers = isRecord(data) && isRecord(data.mcp) ? data.mcp : undefined;
  if (!servers) return [];
  return Object.entries(servers).flatMap(([name, v]) => {
    if (!isRecord(v)) return [];
    const command = Array.isArray(v.command) ? v.command.filter((a): a is string => typeof a === 'string').join(' ') : str(v.command);
    const url = str(v.url);
    const kind = str(v.type);
    const transport = kind === 'local' ? 'stdio' : kind === 'remote' ? 'http' : transportOf(undefined, command, url);
    return [{ name, transport, target: command ?? url ?? '', enabled: v.enabled !== false }];
  });
}

function tomlString(v: string): string {
  const m = /^"((?:[^"\\]|\\.)*)"|^'([^']*)'/.exec(v.trim());
  return m ? (m[1] ?? m[2] ?? '').replace(/\\"/g, '"') : v.trim();
}

function transportOf(kind: string | undefined, command?: string, url?: string): McpTransport {
  const k = kind?.toLowerCase();
  if (k === 'sse') return 'sse';
  if (k === 'stdio') return 'stdio';
  if (k === 'http' || k === 'streamable-http' || k === 'streamable_http') return 'http';
  return command ? 'stdio' : url ? 'http' : 'stdio';
}

// JSON with `//` and `/* */` comments and trailing commas (Devin's config files allow them); strict JSON is tried first
export function parseJsonLoose(text: string): unknown {
  try { return JSON.parse(text); } catch { /* fall through */ }
  let out = '';
  let i = 0;
  while (i < text.length) {
    const c = text[i]!;
    if (c === '"') {
      let j = i + 1;
      while (j < text.length && text[j] !== '"') { if (text[j] === '\\') j++; j++; }
      out += text.slice(i, j + 1);
      i = j + 1;
    } else if (c === '/' && text[i + 1] === '/') {
      while (i < text.length && text[i] !== '\n') i++;
    } else if (c === '/' && text[i + 1] === '*') {
      const end = text.indexOf('*/', i + 2);
      i = end < 0 ? text.length : end + 2;
    } else { out += c; i++; }
  }
  out = out.replace(/,(\s*[}\]])/g, '$1');
  try { return JSON.parse(out); } catch { return undefined; }
}

// Skills: <dir>/<name>/SKILL.md with a YAML-ish frontmatter carrying name / description

async function readSkills(template: string, env: ScanEnv): Promise<InventorySkill[]> {
  const dir = expandPath(template, env);
  let entries: import('node:fs').Dirent[];
  try { entries = await readdir(dir, { withFileTypes: true }); } catch { return []; }
  const scope = scopeOf(template);
  const found = await Promise.all(entries.map(async (d): Promise<InventorySkill | undefined> => {
    if (d.isDirectory() || d.isSymbolicLink()) {
      const path = join(dir, d.name, 'SKILL.md');
      try {
        const fm = parseFrontmatter(await readFile(path, 'utf8'));
        return { name: fm.name ?? d.name, description: fm.description, path, scope };
      } catch { return undefined; }
    }
    // DSH also accepts a flat <name>.md next to the bundles; it needs a frontmatter name to count
    if (d.isFile() && /\.md$/i.test(d.name)) {
      const path = join(dir, d.name);
      try {
        const fm = parseFrontmatter(await readFile(path, 'utf8'));
        return fm.name ? { name: fm.name, description: fm.description, path, scope } : undefined;
      } catch { return undefined; }
    }
    return undefined;
  }));
  return found.filter((s): s is InventorySkill => s !== undefined).sort((a, b) => a.name.localeCompare(b.name));
}

// Only `key: value` lines between the first two `---` fences; folded / literal block scalars take their following indented lines
export function parseFrontmatter(text: string): { name?: string; description?: string } {
  const m = /^---\r?\n([\s\S]*?)\r?\n---/.exec(text);
  if (!m) return {};
  const out: Record<string, string> = {};
  const lines = m[1]!.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const kv = /^([A-Za-z_][\w-]*):\s*(.*)$/.exec(lines[i]!);
    if (!kv) continue;
    let value = kv[2]!.trim();
    if (value === '>' || value === '|' || value === '>-' || value === '|-') {
      const block: string[] = [];
      while (i + 1 < lines.length && /^\s+\S/.test(lines[i + 1]!)) block.push(lines[++i]!.trim());
      value = block.join(value.startsWith('>') ? ' ' : '\n');
    }
    out[kv[1]!] = unquote(value);
  }
  return { name: out.name, description: out.description };
}

function unquote(v: string): string {
  const m = /^"((?:[^"\\]|\\.)*)"$|^'([^']*)'$/.exec(v);
  return m ? (m[1] ?? m[2] ?? '').replace(/\\"/g, '"') : v;
}

// Rules: single files, or directories of *.md / *.mdc

async function readRules(source: RuleSource, env: ScanEnv): Promise<InventoryFile[]> {
  if (!source.dir) return [await fileInfo(source.path, env)];
  const dir = expandPath(source.path, env);
  const scope = scopeOf(source.path);
  try {
    const names = (await readdir(dir, { withFileTypes: true })).filter(d => d.isFile() && /\.(md|mdc)$/i.test(d.name)).map(d => d.name).sort();
    return Promise.all(names.map(async n => { const path = join(dir, n); const s = await stat(path); return { path, scope, exists: true, size: s.size }; }));
  } catch { return []; }
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v ? v : undefined;
}
