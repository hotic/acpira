import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';

// Zero-model-call offline replayer for built-in agent JSONL session logs (`store.rs`).
// Measures:
//   1. Theoretical token savings for `read` line-number prefix A/B arms, `grep` per-file grouping,
//      and selective `bash` build/test/install noise folding.
//   2. Inter-request cache gap behavior around the Anthropic 300s ephemeral TTL.
//   3. Tool guardrail baselines (`old_string not found`, `outputs/` rereads, repeated bash commands).
//
//   pnpm exec tsx scripts/eval/replay.ts [PATH_OR_DIR…] [--limit N] [--json]

const BYTES_PER_TOKEN = 3.4;

interface ToolResultItem {
  role: 'tool';
  id: string;
  name: string;
  content: string;
  error?: boolean;
}

interface AssistantItem {
  role: 'assistant';
  calls?: { id: string; name: string; arguments: string }[];
}

interface RequestEvent {
  type: 'request';
  t?: number;
  model?: string;
  format?: string;
  ms?: number;
  usage?: { input?: number; output?: number; cacheRead?: number; cacheWrite?: number };
}

interface GapBucket {
  calls: number;
  cacheReadSum: number;
  cacheWriteSum: number;
  inputSum: number;
  cacheDrops: number;
}

interface ReplaySummary {
  files: number;
  toolResults: { read: number; grep: number; bash: number; edit: number };
  readBytes: { baseline: number; compact: number; sparse10: number; stripped: number };
  grepBytes: { baseline: number; grouped: number };
  bashBytes: { baseline: number; folded: number; eligibleCalls: number };
  guardrails: { editCalls: number; editNotFound: number; outputRereads: number; bashRepeats: number };
  cacheGaps: Record<string, Record<'le60s' | 's60_180' | 's180_300' | 'gt300s', GapBucket>>;
}

const LINE_NUM_RE = /^ *(\d+)\t(.*)$/;
const GREP_LINE_RE = /^([^:\n]+):(\d+):(.*)$/;
const BASH_RAW_KEEP_RE = /(^|[;&|]\s*)(cat|git\s+diff|git\s+show|jq|awk|sed|python3?\s+-c)\b/;
const BASH_BUILD_TEST_RE = /\b(cargo\s+(test|check|clippy|build)|pnpm|npm|yarn|pytest|unittest|go\s+test|gradle|gradlew|swift\s+(test|build)|xcodebuild)\b/;
const NOISE_LINE_RE = /^\s*(Compiling|Downloading|Downloaded|Fetching|Fresh|Checking|Updating|Blocking waiting|Locking|Packaging|Archiving|PASS\s|ok\s+|test\s+\S+\s+\.\.\.\s+ok|✓\s|✔\s|added \d+ packages|Progress:|\[\d+\/\d+\]|> Task :)/;
const DIAG_LINE_RE = /\b(error|ERROR|FAILED|FAIL\b|panic|panicked|warning:|AssertionError|Traceback|Exception|exit code|timed out)\b/;

export function transformReadLines(content: string): { compact: string; sparse10: string; stripped: string; matchedLines: number } {
  const lines = content.split('\n');
  const compact: string[] = [];
  const sparse10: string[] = [];
  const stripped: string[] = [];
  let matchedLines = 0;

  for (const line of lines) {
    const m = LINE_NUM_RE.exec(line);
    if (!m) {
      compact.push(line);
      sparse10.push(line);
      stripped.push(line);
      continue;
    }
    matchedLines++;
    const num = Number(m[1]);
    const body = m[2]!;
    compact.push(`${num}\t${body}`);
    sparse10.push(num === 1 || num % 10 === 0 ? `${num}\t${body}` : body);
    stripped.push(body);
  }
  return {
    compact: compact.join('\n'),
    sparse10: sparse10.join('\n'),
    stripped: stripped.join('\n'),
    matchedLines,
  };
}

// Lossless per-file grouping for context=0 grep output (`path:line:text` -> `path\nline:text`)
export function groupGrepOutput(content: string): string {
  const lines = content.split('\n');
  const out: string[] = [];
  let currentFile = '';
  let grouped = 0;

  for (const line of lines) {
    const m = GREP_LINE_RE.exec(line);
    if (!m) {
      currentFile = '';
      out.push(line);
      continue;
    }
    const [, file, lineNo, text] = m as unknown as [string, string, string, string];
    if (file !== currentFile) {
      currentFile = file;
      out.push(file);
    } else {
      grouped++;
    }
    out.push(`${lineNo}:${text}`);
  }
  // Only replace if grouping actually saves bytes
  const joined = out.join('\n');
  return grouped > 0 && joined.length < content.length ? joined : content;
}

// Selective folding for recognized build/test/install stdout: keeps diagnostics, failures, and tail
export function foldBuildTestOutput(command: string, content: string): { folded: string; eligible: boolean } {
  if (!command || BASH_RAW_KEEP_RE.test(command) || !BASH_BUILD_TEST_RE.test(command)) {
    return { folded: content, eligible: false };
  }
  const lines = content.split('\n');
  if (lines.length < 24) return { folded: content, eligible: true };

  const out: string[] = [];
  let foldedRun = 0;
  const tailKeep = 12;
  const cutoff = Math.max(0, lines.length - tailKeep);

  const flush = () => {
    if (foldedRun > 0) {
      out.push(`[... ${foldedRun} repetitive progress/pass lines folded ...]`);
      foldedRun = 0;
    }
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!;
    if (i < cutoff && NOISE_LINE_RE.test(line) && !DIAG_LINE_RE.test(line)) {
      foldedRun++;
    } else {
      flush();
      out.push(line);
    }
  }
  flush();
  const joined = out.join('\n');
  return { folded: joined.length < content.length ? joined : content, eligible: true };
}

function emptyBucket(): GapBucket {
  return { calls: 0, cacheReadSum: 0, cacheWriteSum: 0, inputSum: 0, cacheDrops: 0 };
}

function familyOf(model: string, format: string): string {
  const s = `${model} ${format}`.toLowerCase();
  if (s.includes('claude') || s.includes('anthropic') || s.includes('opus') || s.includes('sonnet') || s.includes('haiku') || s.includes('fable')) return 'anthropic';
  if (s.includes('gpt') || s.includes('codex') || s.includes('o1') || s.includes('o3')) return 'openai';
  if (s.includes('gemini')) return 'gemini';
  return 'other';
}

function collectJsonlFiles(inputs: string[], limit: number): string[] {
  const out: { path: string; mtime: number }[] = [];
  const visit = (p: string) => {
    if (!existsSync(p)) return;
    const st = statSync(p);
    if (st.isFile() && p.endsWith('.jsonl')) {
      out.push({ path: p, mtime: st.mtimeMs });
    } else if (st.isDirectory()) {
      for (const entry of readdirSync(p, { withFileTypes: true })) {
        if (entry.isFile() && entry.name.endsWith('.jsonl')) {
          const fp = join(p, entry.name);
          out.push({ path: fp, mtime: statSync(fp).mtimeMs });
        } else if (entry.isDirectory() && (entry.name === 'sessions' || entry.name === 'agent' || entry.name === 'acpira' || entry.name === 'runs')) {
          visit(join(p, entry.name));
        }
      }
    }
  };
  for (const inp of inputs) visit(resolve(inp));
  out.sort((a, b) => b.mtime - a.mtime);
  return out.slice(0, limit).map(x => x.path);
}

export function replayFiles(files: string[]): ReplaySummary {
  const summary: ReplaySummary = {
    files: files.length,
    toolResults: { read: 0, grep: 0, bash: 0, edit: 0 },
    readBytes: { baseline: 0, compact: 0, sparse10: 0, stripped: 0 },
    grepBytes: { baseline: 0, grouped: 0 },
    bashBytes: { baseline: 0, folded: 0, eligibleCalls: 0 },
    guardrails: { editCalls: 0, editNotFound: 0, outputRereads: 0, bashRepeats: 0 },
    cacheGaps: {},
  };

  for (const file of files) {
    const raw = readFileSync(file, 'utf8');
    const callArgs = new Map<string, { name: string; args: Record<string, unknown> }>();
    const seenBash = new Set<string>();
    let lastReqT: number | undefined;
    let lastReqModel = '';
    let prevHitRate = 0;

    for (const line of raw.split('\n')) {
      if (!line) continue;
      let ev: Record<string, unknown>;
      try {
        ev = JSON.parse(line) as Record<string, unknown>;
      } catch {
        continue;
      }

      if (ev.type === 'request') {
        const req = ev as unknown as RequestEvent;
        const model = String(req.model ?? '');
        const fam = familyOf(model, String(req.format ?? ''));
        const u = req.usage ?? {};
        const input = Number(u.input ?? 0);
        const cacheRead = Number(u.cacheRead ?? 0);
        const cacheWrite = Number(u.cacheWrite ?? 0);
        const hitRate = input > 0 ? cacheRead / input : 0;

        if (typeof req.t === 'number' && lastReqT !== undefined && model === lastReqModel) {
          const gapS = (req.t - lastReqT) / 1000;
          if (gapS >= 0) {
            const key = gapS <= 60 ? 'le60s' : gapS <= 180 ? 's60_180' : gapS <= 300 ? 's180_300' : 'gt300s';
            summary.cacheGaps[fam] ??= { le60s: emptyBucket(), s60_180: emptyBucket(), s180_300: emptyBucket(), gt300s: emptyBucket() };
            const b = summary.cacheGaps[fam]![key];
            b.calls++;
            b.inputSum += input;
            b.cacheReadSum += cacheRead;
            b.cacheWriteSum += cacheWrite;
            if (prevHitRate >= 0.5 && hitRate < 0.25 && input >= 4000) b.cacheDrops++;
          }
        }
        if (typeof req.t === 'number') lastReqT = req.t;
        lastReqModel = model;
        prevHitRate = hitRate;
        continue;
      }

      if (ev.type !== 'item' || typeof ev.item !== 'object' || !ev.item) continue;
      const item = ev.item as Record<string, unknown>;

      if (item.role === 'assistant') {
        const a = item as unknown as AssistantItem;
        for (const c of a.calls ?? []) {
          let parsed: Record<string, unknown> = {};
          try {
            parsed = JSON.parse(c.arguments) as Record<string, unknown>;
          } catch {
            // Keep empty args on malformed JSON
          }
          callArgs.set(c.id, { name: c.name, args: parsed });
          if (c.name === 'bash' || c.name === 'run_command') {
            const cmd = String(parsed.command ?? '').trim();
            if (cmd) {
              if (seenBash.has(cmd)) summary.guardrails.bashRepeats++;
              else seenBash.add(cmd);
            }
          } else if (c.name === 'read') {
            const paths = [
              ...(typeof parsed.path === 'string' ? [parsed.path] : []),
              ...(Array.isArray(parsed.paths) ? parsed.paths.filter((x): x is string => typeof x === 'string') : []),
            ];
            for (const p of paths) {
              if (p.includes('/outputs/') || p.startsWith('outputs/')) summary.guardrails.outputRereads++;
            }
          }
        }
        continue;
      }

      if (item.role === 'tool') {
        const tr = item as unknown as ToolResultItem;
        const content = String(tr.content ?? '');
        const call = callArgs.get(tr.id);
        const name = tr.name || call?.name || '';

        if (name === 'read') {
          summary.toolResults.read++;
          const baseLen = Buffer.byteLength(content, 'utf8');
          const t = transformReadLines(content);
          summary.readBytes.baseline += baseLen;
          summary.readBytes.compact += Buffer.byteLength(t.compact, 'utf8');
          summary.readBytes.sparse10 += Buffer.byteLength(t.sparse10, 'utf8');
          summary.readBytes.stripped += Buffer.byteLength(t.stripped, 'utf8');
        } else if (name === 'grep') {
          summary.toolResults.grep++;
          const baseLen = Buffer.byteLength(content, 'utf8');
          const grouped = groupGrepOutput(content);
          summary.grepBytes.baseline += baseLen;
          summary.grepBytes.grouped += Buffer.byteLength(grouped, 'utf8');
        } else if (name === 'bash') {
          summary.toolResults.bash++;
          const cmd = String(call?.args.command ?? '');
          const baseLen = Buffer.byteLength(content, 'utf8');
          const { folded, eligible } = foldBuildTestOutput(cmd, content);
          summary.bashBytes.baseline += baseLen;
          summary.bashBytes.folded += Buffer.byteLength(folded, 'utf8');
          if (eligible) summary.bashBytes.eligibleCalls++;
        } else if (name === 'edit') {
          summary.toolResults.edit++;
          summary.guardrails.editCalls++;
          if (tr.error && /old_string not found|not found in/i.test(content)) {
            summary.guardrails.editNotFound++;
          }
        }
      }
    }
  }
  return summary;
}

const pct = (saved: number, base: number) => (base > 0 ? `${((saved / base) * 100).toFixed(2)}%` : '0.00%');
const tok = (bytes: number) => `${(bytes / BYTES_PER_TOKEN / 1000).toFixed(1)}k tok`;

if (import.meta.url === `file://${process.argv[1]}`) {
  const args = process.argv.slice(2);
  const asJson = args.includes('--json');
  const limitIdx = args.indexOf('--limit');
  const limit = limitIdx >= 0 ? Number(args[limitIdx + 1] ?? '50') : 50;
  const targets = args.filter((a, i) => !a.startsWith('--') && (limitIdx < 0 || i !== limitIdx + 1));
  const inputs = targets.length ? targets : [join(homedir(), '.acpira/agent/sessions')];
  const files = collectJsonlFiles(inputs, limit);
  const s = replayFiles(files);

  if (asJson) {
    console.log(JSON.stringify(s, null, 2));
  } else {
    console.log(`Replayed ${s.files} session JSONL file(s)`);
    console.log(`\n[1] read line-number prefix A/B (${s.toolResults.read} calls, baseline ${tok(s.readBytes.baseline)}):`);
    console.log(`  - compact ({n}\\t):      save ${tok(s.readBytes.baseline - s.readBytes.compact)} (${pct(s.readBytes.baseline - s.readBytes.compact, s.readBytes.baseline)})`);
    console.log(`  - sparse-10 (every 10): save ${tok(s.readBytes.baseline - s.readBytes.sparse10)} (${pct(s.readBytes.baseline - s.readBytes.sparse10, s.readBytes.baseline)})`);
    console.log(`  - stripped (no nums):   save ${tok(s.readBytes.baseline - s.readBytes.stripped)} (${pct(s.readBytes.baseline - s.readBytes.stripped, s.readBytes.baseline)})`);

    console.log(`\n[2] grep per-file grouping (${s.toolResults.grep} calls, baseline ${tok(s.grepBytes.baseline)}):`);
    console.log(`  - grouped:              save ${tok(s.grepBytes.baseline - s.grepBytes.grouped)} (${pct(s.grepBytes.baseline - s.grepBytes.grouped, s.grepBytes.baseline)})`);

    console.log(`\n[3] bash selective build/test folding (${s.bashBytes.eligibleCalls}/${s.toolResults.bash} eligible calls, baseline ${tok(s.bashBytes.baseline)}):`);
    console.log(`  - folded:               save ${tok(s.bashBytes.baseline - s.bashBytes.folded)} (${pct(s.bashBytes.baseline - s.bashBytes.folded, s.bashBytes.baseline)})`);

    console.log(`\n[4] Tool guardrails baseline:`);
    console.log(`  - edit old_string miss: ${s.guardrails.editNotFound}/${s.guardrails.editCalls} (${pct(s.guardrails.editNotFound, s.guardrails.editCalls)})`);
    console.log(`  - outputs/ rereads:     ${s.guardrails.outputRereads}`);
    console.log(`  - repeated bash cmds:   ${s.guardrails.bashRepeats}`);

    console.log(`\n[5] Inter-request cache gap buckets (300s TTL boundary):`);
    for (const [fam, buckets] of Object.entries(s.cacheGaps)) {
      console.log(`  family=${fam}`);
      for (const [k, b] of Object.entries(buckets)) {
        if (!b.calls) continue;
        const hit = b.inputSum > 0 ? ((b.cacheReadSum / b.inputSum) * 100).toFixed(1) : '0.0';
        const avgWrite = Math.round(b.cacheWriteSum / b.calls);
        console.log(`    ${k.padEnd(8)} calls=${String(b.calls).padEnd(5)} cacheHit=${hit.padStart(5)}% avgWrite=${String(avgWrite).padEnd(7)} drops=${b.cacheDrops}`);
      }
    }
  }
}
