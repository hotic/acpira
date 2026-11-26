import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const ROOT = fileURLToPath(new URL('..', import.meta.url));

function walk(dir: string, filter: (full: string, name: string) => boolean): string[] {
  if (!existsSync(dir)) return [];
  const out: string[] = [];
  for (const name of readdirSync(dir).sort()) {
    const full = join(dir, name);
    const st = statSync(full);
    if (st.isDirectory()) {
      if (name === 'node_modules' || name === 'target' || name === 'dist' || name === 'test' || name === '__tests__') continue;
      out.push(...walk(full, filter));
    } else if (filter(full, name)) {
      out.push(full);
    }
  }
  return out;
}

// Tarjan strongly connected components with size > 1 (cycles)
function findSccs(graph: Map<string, Set<string>>): string[][] {
  const nodes = new Set<string>(graph.keys());
  for (const targets of graph.values()) for (const t of targets) nodes.add(t);
  let nextIndex = 0;
  const index = new Map<string, number>();
  const low = new Map<string, number>();
  const stack: string[] = [];
  const onStack = new Set<string>();
  const sccs: string[][] = [];

  function visit(v: string) {
    index.set(v, nextIndex);
    low.set(v, nextIndex);
    nextIndex++;
    stack.push(v);
    onStack.add(v);
    for (const w of graph.get(v) ?? []) {
      if (!index.has(w)) {
        visit(w);
        low.set(v, Math.min(low.get(v)!, low.get(w)!));
      } else if (onStack.has(w)) {
        low.set(v, Math.min(low.get(v)!, index.get(w)!));
      }
    }
    if (low.get(v) === index.get(v)) {
      const comp: string[] = [];
      while (true) {
        const w = stack.pop()!;
        onStack.delete(w);
        comp.push(w);
        if (w === v) break;
      }
      if (comp.length > 1) sccs.push(comp.sort());
    }
  }

  for (const n of [...nodes].sort()) {
    if (!index.has(n)) visit(n);
  }
  return sccs;
}

// Rust source scanner: strips comments/strings, skips #[cfg(test)] items by
// brace/semicolon matching (preserving production items that follow), parses
// `use` trees (crate::, super::, self::, external acpira_*::) and inline paths.

function stripRustNoise(src: string): string {
  const out: string[] = [];
  let i = 0;
  const n = src.length;
  while (i < n) {
    const ch = src[i]!;
    const next = src[i + 1];
    // Line comment
    if (ch === '/' && next === '/') {
      while (i < n && src[i] !== '\n') i++;
      continue;
    }
    // Nested block comment
    if (ch === '/' && next === '*') {
      let depth = 1;
      i += 2;
      while (i < n && depth > 0) {
        if (src[i] === '/' && src[i + 1] === '*') { depth++; i += 2; }
        else if (src[i] === '*' && src[i + 1] === '/') { depth--; i += 2; }
        else { if (src[i] === '\n') out.push('\n'); i++; }
      }
      out.push(' ');
      continue;
    }
    // Raw string literal: r"...", r#"..."#, br#"..."#, cr#"..."#
    if ((ch === 'r' || ((ch === 'b' || ch === 'c') && next === 'r')) && (i === 0 || !/[A-Za-z0-9_]/.test(src[i - 1]!))) {
      let j = ch === 'r' ? i + 1 : i + 2;
      let hashes = 0;
      while (j < n && src[j] === '#') { hashes++; j++; }
      if (src[j] === '"') {
        j++;
        const end = '"' + '#'.repeat(hashes);
        const close = src.indexOf(end, j);
        const target = close === -1 ? n : close + end.length;
        for (let k = i; k < target; k++) if (src[k] === '\n') out.push('\n');
        out.push('""');
        i = target;
        continue;
      }
    }
    // Normal / byte string literal
    if (ch === '"') {
      i++;
      while (i < n && src[i] !== '"') {
        if (src[i] === '\\') i += 2;
        else { if (src[i] === '\n') out.push('\n'); i++; }
      }
      if (i < n) i++;
      out.push('""');
      continue;
    }
    // Char literal (distinguish from lifetimes like 'a or 'static)
    if (ch === '\'') {
      if (next === '\\') {
        let j = i + 2;
        while (j < n && src[j] !== '\'' && src[j] !== '\n') j++;
        if (j < n && src[j] === '\'') { out.push("''"); i = j + 1; continue; }
      } else if (i + 2 < n && src[i + 2] === '\'' && next !== '\n') {
        out.push("''");
        i += 3;
        continue;
      }
    }
    out.push(ch);
    i++;
  }
  return out.join('');
}

// Removes items guaranteed disabled in all non-test production builds (e.g. #[cfg(test)], #[cfg(all(..., test))],
// #[cfg(any(test, feature = "mock"))]) while preserving cross-platform production items like #[cfg(any(windows, test))].
function splitTopLevelCommas(s: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let start = 0;
  for (let i = 0; i < s.length; i++) {
    if (s[i] === '(') depth++;
    else if (s[i] === ')') depth--;
    else if (s[i] === ',' && depth === 0) {
      parts.push(s.slice(start, i));
      start = i + 1;
    }
  }
  parts.push(s.slice(start));
  return parts.map(p => p.trim()).filter(Boolean);
}

function canBeActiveInProd(expr: string): boolean {
  const s = expr.trim();
  if (!s) return true;
  if (s === 'test' || s === 'feature="mock"') return false;
  if (s.startsWith('not(') && s.endsWith(')')) return true;
  if (s.startsWith('all(') && s.endsWith(')')) {
    return splitTopLevelCommas(s.slice(4, -1)).every(canBeActiveInProd);
  }
  if (s.startsWith('any(') && s.endsWith(')')) {
    return splitTopLevelCommas(s.slice(4, -1)).some(canBeActiveInProd);
  }
  return true;
}

function stripRustTestItems(clean: string): { code: string; testMods: Set<string> } {
  const testMods = new Set<string>();
  const out: string[] = [];
  let i = 0;
  const n = clean.length;

  function isTestCfgAttr(attr: string): boolean {
    const compact = attr.replace(/\s+/g, '');
    if (!compact.startsWith('#[cfg(') || !compact.endsWith(')]')) return false;
    return !canBeActiveInProd(compact.slice('#[cfg('.length, -2));
  }

  function skipBalanced(pos: number, open: string, close: string): number {
    let depth = 0;
    let p = pos;
    while (p < n) {
      if (clean[p] === open) depth++;
      else if (clean[p] === close) {
        depth--;
        if (depth === 0) return p + 1;
      }
      p++;
    }
    return n;
  }

  while (i < n) {
    if (clean[i] === '#' && clean[i + 1] === '[') {
      const endAttr = skipBalanced(i + 1, '[', ']');
      const attr = clean.slice(i, endAttr);
      if (isTestCfgAttr(attr)) {
        // Advance through any stacked attributes before the annotated item
        let j = endAttr;
        while (j < n) {
          while (j < n && /\s/.test(clean[j]!)) j++;
          if (clean[j] === '#' && clean[j + 1] === '[') {
            j = skipBalanced(j + 1, '[', ']');
          } else {
            break;
          }
        }
        // Check if this is an out-of-line `mod foo;` test module
        const modDecl = /^(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z0-9_]+)\s*;/.exec(clean.slice(j));
        if (modDecl) {
          testMods.add(modDecl[1]!);
          i = j + modDecl[0].length;
          continue;
        }
        // Skip item until matching `{ ... }` or terminating `;`
        while (j < n && clean[j] !== '{' && clean[j] !== ';') j++;
        if (j < n && clean[j] === '{') j = skipBalanced(j, '{', '}');
        else if (j < n && clean[j] === ';') j++;
        i = j;
        continue;
      }
    }
    out.push(clean[i]!);
    i++;
  }
  return { code: out.join(''), testMods };
}

function expandRustUseTree(prefix: string[], tree: string): string[][] {
  const s = tree.trim();
  if (!s) return [];
  if (s.startsWith('{') && s.endsWith('}')) {
    const inner = s.slice(1, -1);
    const parts: string[] = [];
    let depth = 0;
    let start = 0;
    for (let i = 0; i < inner.length; i++) {
      if (inner[i] === '{') depth++;
      else if (inner[i] === '}') depth--;
      else if (inner[i] === ',' && depth === 0) {
        parts.push(inner.slice(start, i));
        start = i + 1;
      }
    }
    parts.push(inner.slice(start));
    return parts.flatMap(p => expandRustUseTree(prefix, p));
  }
  const braceIdx = s.indexOf('{');
  if (braceIdx !== -1) {
    const head = s.slice(0, braceIdx).replace(/::\s*$/, '').trim();
    const headParts = head ? head.split('::').map(x => x.trim()).filter(Boolean) : [];
    const rest = s.slice(braceIdx).trim();
    return expandRustUseTree([...prefix, ...headParts], rest);
  }
  // Drop `as alias`
  const noAlias = s.replace(/\s+as\s+[A-Za-z0-9_]+$/, '').trim();
  const segs = noAlias.split('::').map(x => x.trim()).filter(Boolean);
  if (segs.length === 0) return [];
  if (segs.length === 1 && segs[0] === 'self') return [prefix];
  return [[...prefix, ...segs]];
}

interface RustRef {
  file: string; // relative to crate src/
  kind: 'internal' | 'external';
  segments: string[]; // resolved from crate root for internal, or [crate_name, ...] for external
}

function rustModulePathOf(relFile: string): string[] {
  const noExt = relFile.replace(/\.rs$/, '');
  const parts = noExt.split('/');
  if (parts[parts.length - 1] === 'mod' || parts[parts.length - 1] === 'lib' || parts[parts.length - 1] === 'main') {
    parts.pop();
  }
  return parts;
}

function scanRustCrateRefs(srcDir: string): { files: string[]; refs: RustRef[] } {
  const allFiles = walk(srcDir, (_full, name) => name.endsWith('.rs'));
  const relFiles = allFiles.map(f => relative(srcDir, f).replace(/\\/g, '/'));
  // Collect test-only out-of-line modules declared under #[cfg(test)] in parent files
  const skippedFiles = new Set<string>();
  const strippedByFile = new Map<string, string>();

  for (const rel of relFiles) {
    const raw = readFileSync(join(srcDir, rel), 'utf8');
    const clean = stripRustNoise(raw);
    const { code, testMods } = stripRustTestItems(clean);
    strippedByFile.set(rel, code);
    if (testMods.size > 0) {
      const parentMod = rustModulePathOf(rel);
      const base = parentMod.length ? parentMod.join('/') + '/' : '';
      for (const tm of testMods) {
        skippedFiles.add(`${base}${tm}.rs`);
        skippedFiles.add(`${base}${tm}/mod.rs`);
      }
    }
  }

  const prodFiles = relFiles.filter(rel => {
    if (skippedFiles.has(rel)) return false;
    for (const sf of skippedFiles) {
      if (sf.endsWith('/mod.rs') && rel.startsWith(sf.slice(0, -'mod.rs'.length))) return false;
    }
    return true;
  });

  const refs: RustRef[] = [];

  for (const rel of prodFiles) {
    const code = strippedByFile.get(rel)!;
    const modPath = rustModulePathOf(rel);

    const pushResolved = (segs: string[]) => {
      if (segs.length === 0) return;
      if (segs[0] === 'crate') {
        if (segs.length > 1) refs.push({ file: rel, kind: 'internal', segments: segs.slice(1) });
        return;
      }
      if (segs[0] === 'super') {
        let up = 0;
        while (up < segs.length && segs[up] === 'super') up++;
        const resolved = [...modPath.slice(0, Math.max(0, modPath.length - up)), ...segs.slice(up)];
        if (resolved.length > 0) refs.push({ file: rel, kind: 'internal', segments: resolved });
        return;
      }
      if (segs[0] === 'self') {
        const resolved = [...modPath, ...segs.slice(1)];
        if (resolved.length > 0) refs.push({ file: rel, kind: 'internal', segments: resolved });
        return;
      }
      if (segs[0]!.startsWith('acpira_')) {
        refs.push({ file: rel, kind: 'external', segments: segs });
      }
    };

    // 1. Parse `use ...;` statements (including `pub use`, `pub(crate) use`)
    const useRe = /\buse\s+([\s\S]*?);/g;
    let m: RegExpExecArray | null;
    while ((m = useRe.exec(code)) !== null) {
      const expanded = expandRustUseTree([], m[1]!);
      for (const pathSegs of expanded) pushResolved(pathSegs);
    }

    // 2. Parse inline qualified paths: crate::..., super::..., self::..., acpira_*::...
    const strippedUses = code.replace(/\buse\s+[\s\S]*?;/g, '');
    const pathRe = /\b((?:crate|super|self|acpira_[a-z0-9_]+)(?:::[A-Za-z0-9_]+)+)/g;
    while ((m = pathRe.exec(strippedUses)) !== null) {
      pushResolved(m[1]!.split('::'));
    }
  }

  return { files: prodFiles, refs };
}

// Cargo.toml dependency graph parser (distinguishes production vs dev-deps, incl. subtables and renamed packages).

function parseCargoProdWorkspaceDeps(cargoTomlPath: string): { name: string; deps: string[] } {
  const txt = readFileSync(cargoTomlPath, 'utf8');
  const name = /^name\s*=\s*"([^"]+)"/m.exec(txt)?.[1] ?? '';
  const deps = new Set<string>();
  let section = '';
  for (const rawLine of txt.split('\n')) {
    const line = rawLine.replace(/#.*$/, '').trim();
    if (!line) continue;
    const sec = /^\[([^\]]+)\]$/.exec(line);
    if (sec) {
      section = sec[1]!.trim();
      const subDep = /^(?:target\..+\.)?dependencies\.(acpira-[a-z0-9_-]+)$/.exec(section);
      if (subDep && subDep[1] !== name) deps.add(subDep[1]!);
      continue;
    }
    const isProdTable = section === 'dependencies' || /^target\..+\.dependencies$/.test(section);
    const isProdSubtable = /^(?:target\..+\.)?dependencies\.[^.]+$/.test(section);
    if (!isProdTable && !isProdSubtable) continue;
    if (isProdTable) {
      const dep = /^(acpira-[a-z0-9_-]+)(?:\.workspace)?\s*=/.exec(line);
      if (dep && dep[1] !== name) deps.add(dep[1]!);
      const renamed = /\bpackage\s*=\s*"(acpira-[a-z0-9_-]+)"/.exec(line);
      if (renamed && renamed[1] !== name) deps.add(renamed[1]!);
    } else {
      const renamed = /^package\s*=\s*"(acpira-[a-z0-9_-]+)"/.exec(line);
      if (renamed && renamed[1] !== name) deps.add(renamed[1]!);
    }
  }
  return { name, deps: [...deps].sort() };
}

// TypeScript import scanner (separates value imports vs type-only imports).

interface TsEdge {
  file: string; // relative to package dir
  targetFile: string; // relative to package dir (resolved, no extension)
  typeOnly: boolean;
}

function resolveTsTarget(packageDir: string, fromFull: string, spec: string, selfPkgName?: string): string | null {
  let base: string;
  if (spec.startsWith('.')) {
    base = resolve(dirname(fromFull), spec);
  } else if (selfPkgName && (spec === selfPkgName || spec.startsWith(`${selfPkgName}/`))) {
    const sub = spec === selfPkgName ? 'index' : spec.slice(selfPkgName.length + 1);
    base = resolve(packageDir, sub);
  } else {
    return null;
  }
  const candidates = [
    base,
    `${base}.ts`,
    `${base}.tsx`,
    join(base, 'index.ts'),
    join(base, 'index.tsx'),
  ];
  for (const c of candidates) {
    if (existsSync(c) && statSync(c).isFile()) {
      const rel = relative(packageDir, c).replace(/\\/g, '/');
      return rel.startsWith('..') ? null : rel;
    }
  }
  return null;
}

function stripTsComments(src: string): string {
  const out: string[] = [];
  let i = 0;
  const n = src.length;
  while (i < n) {
    const ch = src[i]!;
    const next = src[i + 1];
    if (ch === '/' && next === '/') {
      while (i < n && src[i] !== '\n') i++;
      continue;
    }
    if (ch === '/' && next === '*') {
      i += 2;
      while (i < n && !(src[i] === '*' && src[i + 1] === '/')) {
        if (src[i] === '\n') out.push('\n');
        i++;
      }
      if (i < n) i += 2;
      out.push(' ');
      continue;
    }
    if (ch === '\'' || ch === '"' || ch === '`') {
      const q = ch;
      out.push(q);
      i++;
      while (i < n && src[i] !== q) {
        if (src[i] === '\\') { out.push(src[i]!, src[i + 1] ?? ''); i += 2; }
        else { out.push(src[i]!); i++; }
      }
      if (i < n) { out.push(src[i]!); i++; }
      continue;
    }
    out.push(ch);
    i++;
  }
  return out.join('');
}

function isAllTypeSpecifiers(clause: string): boolean {
  const brace = /\{([\s\S]*)\}/.exec(clause);
  if (!brace) return false;
  // If there is a default import outside `{ ... }` (e.g. `import Foo, { type Bar }`), it is a value import.
  const before = clause.slice(0, brace.index).replace(/,/g, '').trim();
  if (before.length > 0) return false;
  const items = brace[1]!.split(',').map(s => s.trim()).filter(Boolean);
  return items.length > 0 && items.every(item => /^type\s+/.test(item));
}

function scanTsPackageEdges(packageDir: string): { files: string[]; edges: TsEdge[] } {
  const pkgJsonPath = join(packageDir, 'package.json');
  const selfPkgName = existsSync(pkgJsonPath)
    ? (JSON.parse(readFileSync(pkgJsonPath, 'utf8')) as { name?: string }).name
    : undefined;
  const fullFiles = walk(packageDir, (_full, name) => /\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) && name !== 'vite.config.ts');
  const files = fullFiles.map(f => relative(packageDir, f).replace(/\\/g, '/'));
  const edges: TsEdge[] = [];

  for (const full of fullFiles) {
    const rel = relative(packageDir, full).replace(/\\/g, '/');
    const clean = stripTsComments(readFileSync(full, 'utf8'));

    const record = (spec: string, typeOnly: boolean) => {
      const target = resolveTsTarget(packageDir, full, spec, selfPkgName);
      if (target && target !== rel) edges.push({ file: rel, targetFile: target, typeOnly });
    };

    // 1. Static import / export ... from '...' (clause restricted to valid specifier characters to prevent bridging across statements)
    const stmtRe = /(?:^|[;\n])\s*(import|export)\s+(type\s+)?([A-Za-z0-9_$,*{}\s]+?)\bfrom\s*['"]([^'"]+)['"]/g;
    let m: RegExpExecArray | null;
    while ((m = stmtRe.exec(clean)) !== null) {
      const hasTypeKw = Boolean(m[2]);
      const clause = m[3] ?? '';
      if (/\b(?:import|export)\b/.test(clause)) continue;
      const spec = m[4]!;
      const typeOnly = hasTypeKw || isAllTypeSpecifiers(clause);
      record(spec, typeOnly);
    }

    // 2. Side-effect import: import './foo';
    const sideEffectRe = /(?:^|[;\n])\s*import\s+['"]([^'"]+)['"]/g;
    while ((m = sideEffectRe.exec(clean)) !== null) {
      record(m[1]!, false);
    }

    // 3. Dynamic import: import('./foo')
    const dynRe = /\bimport\s*\(\s*['"]([^'"]+)['"]\s*\)/g;
    while ((m = dynRe.exec(clean)) !== null) {
      record(m[1]!, false);
    }
  }
  return { files, edges };
}

// Layer Maps & Ratchets (ratchets shrink to [] as each refactoring phase lands).

function hostModuleOf(segments: string[]): string {
  const first = segments[0] ?? '';
  if (first === 'acp') {
    return segments[1] ? `acp/${segments[1]}` : 'acp/mod';
  }
  return first;
}

function hostModuleOfFile(relFile: string): string {
  const parts = relFile.replace(/\.rs$/, '').split('/');
  if (parts[0] === 'acp') {
    return parts[1] ? `acp/${parts[1]}` : 'acp/mod';
  }
  return parts[0]!;
}

// Complete module -> architectural layer map for rust/crates/acpira-host/src.
// Any newly added top-level module MUST be placed here or the test fails.
const HOST_LAYERS: Record<string, number> = {
  // L00: host-base foundation primitives
  util: 0, json: 0, limits: 0, platform: 0, net_proxy: 0, http: 0, i18n: 0, file_rank: 0, agent_ext: 0,
  // L01: persistence & catalogues
  store: 1, model_catalog: 1, bus: 1, inventory: 1,
  // L02: agent process launcher & pure ACP transport
  'acp/launch': 2, 'acp/login_path': 2, 'acp/transport': 2,
  // L03: vendor wire quirks & local-run stand profiles
  'acp/vendors': 3, 'acp/local_run': 3,
  // L04: agent registry, spawn, config, pool & installer
  'acp/agents': 4,
  // L05: account isolation, vault & remote seam
  accounts: 5, remote_seam: 5,
  // L06: providers, shared config, handoff & local hooks
  providers: 6, shared_config: 6, handoff: 6, hooks: 6,
  // L07: transcript normalization, probing, usage analytics, relay wire/roster & workspace seam
  'acp/transcript': 7, 'acp/probe': 7, usage_index: 7, relay_wire: 7, relay_roster: 7, project_files: 7, node_files: 7, workspace: 7,
  // L08: single-session state machine
  'acp/session': 8,
  // L09: remote connections, terminals & preview proxy
  remote: 9, terminal: 9, preview: 9,
  // L10: user plugins runtime & host
  plugins: 10,
  // L11: agent-facing tool gateway
  session_library: 11, computer: 11, relay: 11, host_mcp: 11,
  // L12: multi-session orchestration, settings & sidecar platform seam
  settings: 12, session_manager: 12, sidecar_platform: 12,
  // L13: machine engine & paired devices host
  machine_engine: 13, devices: 13,
  // L14: multi-window bridge & composition root
  bridge_core: 14, runtime: 14,
  // L15: top-level daemons & CLI entrypoints
  sidecar: 15, sidecar_server: 15, remote_helper: 15, agents_cli: 15, cli: 15, bin: 15, lib: 15, 'acp/mod': 15,
};

// Complete top-level module -> layer map for packages/ui.
const UI_LAYERS: Record<string, number> = {
  // U0 (0): design primitives, chrome, effects, host transport & pluginSlots
  i18n: 0, look: 0, appearance: 0, vscodeApi: 0, desktop: 0, keybindings: 0, requestTable: 0,
  probeUrl: 0, registerProperties: 0, fontPreload: 0, sessionViews: 0, useVsCodeTheme: 0,
  ui: 0, chrome: 0, effects: 0, pluginSlots: 0,
  // U1 (10..11): shared rendering & feature foundations (markdown/context/remote/pluginTheme -> plugins)
  markdown: 10, context: 10, remote: 10, pluginTheme: 10,
  plugins: 11,
  // U2 (20..22): core feature partial order (chat -> panel / usage / trace / browser)
  chat: 20,
  panel: 22, usage: 22, trace: 22, browser: 22,
  // U3 (30..31): workspace shell -> settings
  shell: 30,
  settings: 31,
  // U4 (40): app entrypoint & welcome overlay
  welcome: 40, App: 40, main: 40,
};

function uiModuleOf(relFile: string): string {
  const first = relFile.split('/')[0]!;
  return first.replace(/\.tsx?$/, '');
}

// Baseline Ratchets — populated in P0, strictly tightened to [] by P5.

const RATCHET_CARGO_FORBIDDEN: string[] = [];

const RATCHET_HOST_UPWARD: string[] = [];

const RATCHET_HOST_INTRA_SCC: string[] = [];

const RATCHET_HOST_EXTERNAL_FORBIDDEN: string[] = [];

const RATCHET_AGENT_SCCS: string[] = [];

const RATCHET_SHARED_TYPE_SCCS: string[] = [];

const RATCHET_UI_UPWARD: string[] = [];

const RATCHET_UI_DIR_SCCS: string[] = [];

const RATCHET_UI_INTRA_RUNTIME_SCCS: string[] = [];

describe('architecture layers (MIT Axiomatic Design lower-triangular gate)', () => {
  it('Rust workspace production crates are strictly acyclic and obey forbidden edges', () => {
    const cratesDir = join(ROOT, 'rust/crates');
    const graph = new Map<string, Set<string>>();
    for (const dir of readdirSync(cratesDir).sort()) {
      const toml = join(cratesDir, dir, 'Cargo.toml');
      if (!existsSync(toml)) continue;
      const { name, deps } = parseCargoProdWorkspaceDeps(toml);
      graph.set(name, new Set(deps));
    }
    expect(findSccs(graph)).toEqual([]);

    const forbidden: string[] = [];
    if (graph.get('acpira-gateway')?.has('acpira-desktop')) {
      forbidden.push('acpira-gateway -> acpira-desktop');
    }
    for (const dep of graph.get('acpira-store') ?? []) {
      if (dep !== 'acpira-shared' && dep !== 'acpira-host-base') {
        forbidden.push(`acpira-store -> ${dep}`);
      }
    }
    for (const dep of graph.get('acpira-plugin-build') ?? []) {
      if (dep !== 'acpira-host-base') {
        forbidden.push(`acpira-plugin-build -> ${dep}`);
      }
    }
    // oxc_* stays isolated in acpira-plugin-build so host edits never pull compiler deps directly
    const hostProdToml = readFileSync(join(cratesDir, 'acpira-host', 'Cargo.toml'), 'utf8').split(/^\[dev-dependencies\]/m)[0] ?? '';
    if (/(?:\boxc_\w+\s*=|dependencies\.oxc_\w+\]|package\s*=\s*"oxc_\w+")/.test(hostProdToml)) {
      forbidden.push('acpira-host -> oxc_*');
    }
    expect(forbidden).toEqual(RATCHET_CARGO_FORBIDDEN);
  });

  const hostScan = (() => {
    const srcDir = join(ROOT, 'rust/crates/acpira-host/src');
    const { files, refs } = scanRustCrateRefs(srcDir);
    const unmapped = new Set<string>();
    for (const rel of files) {
      const mod = hostModuleOfFile(rel);
      if (!(mod in HOST_LAYERS)) unmapped.add(mod);
    }
    const upward = new Set<string>();
    const externalForbidden = new Set<string>();
    const intraByLayer = new Map<number, Map<string, Set<string>>>();

    for (const r of refs) {
      const srcMod = hostModuleOfFile(r.file);
      const srcLayer = HOST_LAYERS[srcMod];
      if (srcLayer === undefined) continue;
      if (r.kind === 'external') {
        const extCrate = r.segments[0]!;
        if ((srcMod === 'store' || srcMod === 'acp/transport') && extCrate === 'acpira_agent') {
          externalForbidden.add(`${r.file} -> ${extCrate}`);
        }
        continue;
      }
      const tgtMod = hostModuleOf(r.segments);
      if (tgtMod === srcMod || !(tgtMod in HOST_LAYERS)) continue;
      const tgtLayer = HOST_LAYERS[tgtMod]!;
      if (tgtLayer > srcLayer) {
        upward.add(`${r.file} -> ${tgtMod}`);
      } else if (tgtLayer === srcLayer) {
        let g = intraByLayer.get(srcLayer);
        if (!g) { g = new Map(); intraByLayer.set(srcLayer, g); }
        let set = g.get(srcMod);
        if (!set) { set = new Set(); g.set(srcMod, set); }
        set.add(tgtMod);
      }
    }
    const intraSccs: string[] = [];
    for (const [layer, g] of [...intraByLayer.entries()].sort((a, b) => a[0] - b[0])) {
      for (const comp of findSccs(g)) {
        intraSccs.push(`L${String(layer).padStart(2, '0')}: ${comp.join(' <-> ')}`);
      }
    }
    return {
      unmapped: [...unmapped].sort(),
      upward: [...upward].sort(),
      intraSccs: intraSccs.sort(),
      externalForbidden: [...externalForbidden].sort(),
    };
  })();

  it('every acpira-host module is assigned an explicit layer', () => {
    expect(hostScan.unmapped).toEqual([]);
  });

  it('acpira-host has zero upward cross-layer edges', () => {
    expect(hostScan.upward).toEqual(RATCHET_HOST_UPWARD);
  });

  it('acpira-host has zero same-layer module SCCs', () => {
    expect(hostScan.intraSccs).toEqual(RATCHET_HOST_INTRA_SCC);
  });

  it('acpira-host store and transport have zero forbidden cross-crate edges', () => {
    expect(hostScan.externalForbidden).toEqual(RATCHET_HOST_EXTERNAL_FORBIDDEN);
  });

  it('acpira-agent modules are acyclic', () => {
    const srcDir = join(ROOT, 'rust/crates/acpira-agent/src');
    const { refs } = scanRustCrateRefs(srcDir);
    const graph = new Map<string, Set<string>>();
    for (const r of refs) {
      if (r.kind !== 'internal') continue;
      const src = r.file.replace(/\.rs$/, '').split('/')[0]!;
      const tgt = r.segments[0]!;
      if (src === 'lib' || tgt === 'lib' || src === tgt) continue;
      let set = graph.get(src);
      if (!set) { set = new Set(); graph.set(src, set); }
      set.add(tgt);
    }
    const sccs = findSccs(graph).map(c => c.join(' <-> '));
    expect(sccs).toEqual(RATCHET_AGENT_SCCS);
  });

  it('TS workspace packages are strictly acyclic', () => {
    const graph = new Map<string, Set<string>>();
    for (const base of ['packages', 'apps']) {
      const dir = join(ROOT, base);
      for (const name of readdirSync(dir).sort()) {
        const pkgPath = join(dir, name, 'package.json');
        if (!existsSync(pkgPath)) continue;
        const pkg = JSON.parse(readFileSync(pkgPath, 'utf8')) as {
          name?: string;
          dependencies?: Record<string, string>;
          peerDependencies?: Record<string, string>;
        };
        if (!pkg.name) continue;
        const deps = new Set<string>();
        for (const sec of [pkg.dependencies, pkg.peerDependencies]) {
          for (const k of Object.keys(sec ?? {})) {
            if (k.startsWith('@acpira/')) deps.add(k);
          }
        }
        graph.set(pkg.name, deps);
      }
    }
    expect(findSccs(graph)).toEqual([]);
  });

  it('packages/shared has zero value cycles and zero type cycles', () => {
    const dir = join(ROOT, 'packages/shared');
    const { edges } = scanTsPackageEdges(dir);
    const valueGraph = new Map<string, Set<string>>();
    const fullGraph = new Map<string, Set<string>>();
    for (const e of edges) {
      let fg = fullGraph.get(e.file);
      if (!fg) { fg = new Set(); fullGraph.set(e.file, fg); }
      fg.add(e.targetFile);
      if (!e.typeOnly) {
        let vg = valueGraph.get(e.file);
        if (!vg) { vg = new Set(); valueGraph.set(e.file, vg); }
        vg.add(e.targetFile);
      }
    }
    expect(findSccs(valueGraph), 'packages/shared runtime value cycles').toEqual([]);
    const typeSccs = findSccs(fullGraph).map(c => c.join(' <-> '));
    expect(typeSccs, 'packages/shared type+value cycles').toEqual(RATCHET_SHARED_TYPE_SCCS);
  });

  const uiScan = (() => {
    const dir = join(ROOT, 'packages/ui');
    const { files, edges } = scanTsPackageEdges(dir);
    const unmapped = new Set<string>();
    for (const f of files) {
      const m = uiModuleOf(f);
      if (!(m in UI_LAYERS)) unmapped.add(m);
    }
    const upward = new Set<string>();
    const dirGraph = new Map<string, Set<string>>();
    const intraValueByMod = new Map<string, Map<string, Set<string>>>();

    for (const e of edges) {
      const srcMod = uiModuleOf(e.file);
      const tgtMod = uiModuleOf(e.targetFile);
      if (srcMod === tgtMod) {
        if (!e.typeOnly) {
          let g = intraValueByMod.get(srcMod);
          if (!g) { g = new Map(); intraValueByMod.set(srcMod, g); }
          let set = g.get(e.file);
          if (!set) { set = new Set(); g.set(e.file, set); }
          set.add(e.targetFile);
        }
        continue;
      }
      let ds = dirGraph.get(srcMod);
      if (!ds) { ds = new Set(); dirGraph.set(srcMod, ds); }
      ds.add(tgtMod);
      const srcLayer = UI_LAYERS[srcMod]!;
      const tgtLayer = UI_LAYERS[tgtMod]!;
      if (tgtLayer > srcLayer) upward.add(`${e.file} -> ${tgtMod}`);
    }
    const dirSccs = findSccs(dirGraph).map(c => c.join(' <-> ')).sort();
    const intraRuntimeSccs: string[] = [];
    for (const [mod, g] of [...intraValueByMod.entries()].sort()) {
      for (const comp of findSccs(g)) {
        intraRuntimeSccs.push(`${mod}: ${comp.join(' <-> ')}`);
      }
    }
    const appImportsPluginSlots = edges.some(e => e.file === 'App.tsx' && e.targetFile === 'pluginSlots.ts' && !e.typeOnly);
    return {
      unmapped: [...unmapped].sort(),
      upward: [...upward].sort(),
      dirSccs,
      intraRuntimeSccs: intraRuntimeSccs.sort(),
      appImportsPluginSlots,
    };
  })();

  it('every packages/ui top-level module is assigned an explicit layer', () => {
    expect(uiScan.unmapped).toEqual([]);
  });

  it('packages/ui has zero upward U0->U4 layer edges', () => {
    expect(uiScan.upward).toEqual(RATCHET_UI_UPWARD);
  });

  it('packages/ui has zero directory SCCs', () => {
    expect(uiScan.dirSccs).toEqual(RATCHET_UI_DIR_SCCS);
  });

  it('packages/ui has zero intra-directory runtime file cycles', () => {
    expect(uiScan.intraRuntimeSccs).toEqual(RATCHET_UI_INTRA_RUNTIME_SCCS);
  });

  it('App.tsx keeps a static value import of pluginSlots.ts so plugin-ui.js stays beside main.js', () => {
    expect(uiScan.appImportsPluginSlots).toBe(true);
  });
});
