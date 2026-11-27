import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const ROOT = fileURLToPath(new URL('..', import.meta.url));

// A source file past this many lines gets split by topic instead of growing further
const LIMIT = 1200;

const SOURCE = /\.(rs|ts|tsx|swift|kt|ets|mjs|js)$/;
// Translation dictionaries and vendored bundles are not hand-written source (golden output is JSON, never scanned)
const SKIP = [/\/i18n\/[a-z]{2}(-[A-Z]{2})?\.ts$/, /\.min\.js$/, /\/Resources\/web\//];

// Files allowed past the limit, each with the reason it stays whole. A stale entry (file gone or back under the limit)
// fails, so a split takes its entry out; a new entry is a decision for review, never a way around the gate
const ALLOWED: Record<string, string> = {
  // Contracts: the bulk is the HostMsg / WebviewMsg enums, which cannot span files, and one file per language keeps
  // the Rust / TS / Swift / Kotlin mirrors easy to diff
  'rust/crates/acpira-shared/src/protocol.rs': 'contract enums',
  'apps/ios/Packages/AcpiraKit/Sources/AcpiraModels/Protocol.swift': 'contract enums',
  // One turn lifecycle (Staging, Accepted, OpenTurn, Settled); splitting it widens every phase struct's fields
  'rust/crates/acpira-host/src/acp/session/prompt.rs': 'single lifecycle',
  // What is left is the shell's wiring and its provider tree; the thread and the self-contained hooks already moved out
  'packages/ui/shell/Shell.tsx': 'composition root',
  // Bundled by esbuild, run by plain node and typechecked by tsc; its prompt branches fall through with shared state.
  // Escaped so test/globalSetup.ts, which greps suites for the fixture's name, does not build the sidecar for this one
  'test/fake\u002dagent.ts': 'test fixture',
  // One @ObservedV2 class whose methods all read its state; ArkTS has no extensions, so it cannot span files the way
  // the iOS AppModel+X.swift files and the Android extension functions do
  'apps/harmony/entry/src/main/ets/store/AppModel.ets': 'single observed class',
};

// Rust is counted without its top-level inline test modules (`#[cfg(test)]` over `mod x {`, up to its closing brace)
function lineCount(file: string): number {
  const text = readFileSync(join(ROOT, file), 'utf8');
  if (text === '') return 0;
  const lines = text.split('\n');
  if (text.endsWith('\n')) lines.pop();
  if (!file.endsWith('.rs')) return lines.length;
  let count = 0;
  for (let i = 0; i < lines.length; i++) {
    if (/^#\[cfg\(test\)\]/.test(lines[i] ?? '')) {
      let j = i + 1;
      while (j < lines.length && /^#\[/.test(lines[j] ?? '')) j++;
      if (j < lines.length && /^(pub(\([^)]*\))? )?mod \w+ \{/.test(lines[j] ?? '')) {
        while (j < lines.length && lines[j] !== '}') j++;
        i = j;
        continue;
      }
    }
    count++;
  }
  return count;
}

function sources(): string[] {
  return execFileSync('git', ['ls-files', '-z'], { cwd: ROOT, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
    .split('\0')
    // A file deleted but not yet staged is still listed: it counts as gone
    .filter(f => SOURCE.test(f) && !SKIP.some(re => re.test(f)) && existsSync(join(ROOT, f)));
}

describe('source file size', () => {
  const files = sources();
  const sizes = new Map(files.map(f => [f, lineCount(f)]));

  it(`no tracked source file grows past ${LIMIT} lines unless it is listed with a reason`, () => {
    const over = [...sizes].filter(([f, n]) => n > LIMIT && !(f in ALLOWED)).map(([f, n]) => `${f} (${n} lines)`);
    expect(over, 'split these by topic instead of growing them').toEqual([]);
  });

  it('every entry gives a reason', () => {
    const bare = Object.entries(ALLOWED).filter(([, why]) => why.trim() === '').map(([f]) => f);
    expect(bare, 'say why these stay whole').toEqual([]);
  });

  it('every listed file still exists and is still past the limit', () => {
    const stale = Object.keys(ALLOWED).filter(f => (sizes.get(f) ?? 0) <= LIMIT);
    expect(stale, 'remove these entries from ALLOWED').toEqual([]);
  });
});
