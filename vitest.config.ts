import { defineConfig } from 'vitest/config';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  resolve: { alias: { '@shared': fileURLToPath(new URL('./src/shared', import.meta.url)) } },
  test: { include: ['test/**/*.test.ts', 'src/webview/**/*.test.ts'], globalSetup: ['test/globalSetup.ts'], testTimeout: 15_000, hookTimeout: 15_000 },
});
