import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const ROOT = fileURLToPath(new URL('..', import.meta.url));

// A source file past this many lines gets split by topic instead of growing further
const LIMIT = 1200;

const SOURCE = /\.(rs|ts|tsx|swift|kt|ets|mjs|js)$/;
// Data, vendored bundles and golden output are not hand-written source
const SKIP = [/^test\/fixtures\//, /\/i18n\//, /\.min\.js$/, /\/Resources\/web\//];

// Files allowed past the limit, each with the reason it stays whole. An entry is removed once its file is split;
// a stale entry (file gone or back under the limit) fails too, so the list only ever shrinks
const ALLOWED: Record<string, string> = {
  // Contracts: the bulk is the HostMsg / WebviewMsg enums, which cannot span files, and one file per language keeps
  // the Rust / TS / Swift / Kotlin mirrors easy to diff
  'rust/crates/acpira-shared/src/protocol.rs': 'contract enums',
  'apps/ios/Packages/AcpiraKit/Sources/AcpiraModels/Protocol.swift': 'contract enums',
  // One turn lifecycle (Staging, Accepted, OpenTurn, Settled); splitting it widens every phase struct's fields
  'rust/crates/acpira-host/src/acp/session/prompt.rs': 'single lifecycle',
  // What is left is the shell's wiring and its provider tree; the thread and the self-contained hooks already moved out
  'packages/ui/shell/Shell.tsx': 'composition root',
  // Under the limit without its inline tests, one topic
  'rust/crates/acpira-host/src/acp/session/hooks.rs': 'single topic',
  // Bundled by esbuild, run by plain node and typechecked by tsc; its prompt branches fall through with shared state.
  // Escaped so test/globalSetup.ts, which greps suites for the fixture's name, does not build the sidecar for this one
  'test/fake\u002dagent.ts': 'test fixture',

  // Split pending
  'apps/harmony/entry/src/main/ets/store/AppModel.ets': 'split pending',
  'apps/ios/Packages/AcpiraKit/Sources/AcpiraUI/Store/AppModel.swift': 'split pending',
  'apps/ios/Packages/AcpiraKit/Sources/AcpiraUI/Sessions/SessionListScreen.swift': 'split pending',
  'apps/harmony/entry/src/main/ets/pages/Usage.ets': 'split pending',
};

function lineCount(file: string): number {
  const text = readFileSync(join(ROOT, file), 'utf8');
  if (text === '') return 0;
  const lines = text.split('\n').length;
  return text.endsWith('\n') ? lines - 1 : lines;
}

function sources(): string[] {
  return execFileSync('git', ['ls-files', '-z'], { cwd: ROOT, encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
    .split('\0')
    .filter(f => SOURCE.test(f) && !SKIP.some(re => re.test(f)));
}

describe('source file size', () => {
  const files = sources();
  const sizes = new Map(files.map(f => [f, lineCount(f)]));

  it(`no tracked source file grows past ${LIMIT} lines unless it is listed with a reason`, () => {
    const over = [...sizes].filter(([f, n]) => n > LIMIT && !(f in ALLOWED)).map(([f, n]) => `${f} (${n} lines)`);
    expect(over, 'split these by topic instead of growing them').toEqual([]);
  });

  it('every listed file still exists and is still past the limit', () => {
    const stale = Object.keys(ALLOWED).filter(f => (sizes.get(f) ?? 0) <= LIMIT);
    expect(stale, 'remove these entries from ALLOWED').toEqual([]);
  });
});
