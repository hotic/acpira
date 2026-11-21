import { readFileSync } from 'node:fs';
import { relative } from 'node:path';
import type { TestProject } from 'vitest/node';
import { sidecarBin } from '../scripts/lib/sidecarBin';
import { bundleFakeAgent } from './fakeAgentBundle';

// Only suites in test/ that drive the Rust sidecar or launch the fake agent need them built.
const NEEDS_SIDECAR = /sidecarShell|sidecarBin|fakeAgentBundle|hostServer|fake-agent/;

// Bring the workspace's debug sidecar and fake agent up to date once per run when any selected suite needs them
// (ACPIRA_SIDECAR_BIN overrides the binary). Pure package unit suites skip the Cargo build altogether.
export default async function setup(project: TestProject) {
  // Vitest keeps CLI file filters on the internal `filenamePattern` field; when absent, globTestSpecifications returns all specs.
  const filters = (project.vitest as { filenamePattern?: string[] }).filenamePattern;
  const specs = await project.vitest.globTestSpecifications(filters);
  const needs = specs.some(spec => {
    const rel = relative(project.config.root, spec.moduleId).replace(/\\/g, '/');
    return rel.startsWith('test/') && NEEDS_SIDECAR.test(readFileSync(spec.moduleId, 'utf8'));
  });
  if (!needs) return;
  sidecarBin({ build: true });
  bundleFakeAgent();
}
