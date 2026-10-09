import { sidecarBin } from '../scripts/lib/sidecarBin';
import { bundleFakeAgent } from './fakeAgentBundle';

// The suites drive the Rust sidecar: bring the workspace's debug build up to date once per run (ACPIRA_SIDECAR_BIN skips this),
// and the fake agent they launch through it
export default function setup() {
  sidecarBin({ build: true });
  bundleFakeAgent();
}
