import { defineConfig } from 'vitest/config';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  resolve: {
    alias: {
      '@shared': fileURLToPath(new URL('../../packages/shared', import.meta.url)),
      '@engine-client': fileURLToPath(new URL('../../packages/engine-client', import.meta.url)),
    },
  },
  test: { include: ['test/**/*.test.ts'] },
});
