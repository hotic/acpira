import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    include: ['test/**/*.test.ts', 'packages/ui/**/*.test.ts', 'packages/picker/**/*.test.ts'],
    globalSetup: ['test/globalSetup.ts'],
    testTimeout: 15_000,
    hookTimeout: 15_000,
    // Every contract suite starts a sidecar and its fake agents: keep a fifth of the machine for the desktop
    maxWorkers: '80%',
  },
});
