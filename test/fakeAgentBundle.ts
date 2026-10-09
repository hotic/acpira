import { buildSync } from 'esbuild';
import { mkdirSync, renameSync } from 'node:fs';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

// test/fake-agent.ts bundled into one ES module that node runs directly: one file per spawn instead of tsx transpiling the
// script and resolving the SDK's modules, which dominated agent start-up with dozens of agents starting in parallel (and on
// Windows every opened file also goes through the antivirus filter). Node itself is the agent process, so SIGTERM / SIGKILL
// reach it without a wrapper in between. The Rust engine suites build their own copy with the same flags (tests/engine/support.rs).
export const FAKE = fileURLToPath(new URL('../node_modules/.cache/acpira-test/fake-agent.mjs', import.meta.url));
const SOURCE = fileURLToPath(new URL('./fake-agent.ts', import.meta.url));

export function bundleFakeAgent() {
  mkdirSync(dirname(FAKE), { recursive: true });
  // Built aside and renamed into place: another run may be starting agents from the current copy
  const tmp = `${FAKE.slice(0, -'.mjs'.length)}.${process.pid}.mjs`;
  buildSync({ entryPoints: [SOURCE], outfile: tmp, bundle: true, platform: 'node', format: 'esm', logLevel: 'warning' });
  renameSync(tmp, FAKE);
}
