import { sidecarBin } from '../scripts/lib/sidecarBin';

// The suites drive the Rust sidecar: bring the workspace's debug build up to date once per run (ACPIRA_SIDECAR_BIN skips this)
export default function setup() {
  sidecarBin({ build: true });
}
