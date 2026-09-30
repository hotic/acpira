import { defineConfig, mergeConfig } from 'vite';
import { fileURLToPath } from 'node:url';
import base from './vite.lab.config.ts';

const here = (path: string) => fileURLToPath(new URL(path, import.meta.url));

// Compile preview-only slots without changing production components or the shared LAB server.
export default mergeConfig(base, defineConfig({
  plugins: [{
    name: 'plan-permissions-preview-slots',
    enforce: 'pre',
    transform(source, id) {
      const file = id.split('?')[0];
      const bridge = `/@fs${here('./lab/plan-permissions.bridge.tsx')}`;
      if (file === here('./src/webview/chat/Composer.tsx')) {
        const target = 'function ModeMenu(';
        if (!source.includes(target)) this.error('Plan preview requires the current Composer ModeMenu slot');
        return {
          code: `import { PlanModeMenu } from '${bridge}';\n${source.replace(target, 'function ProductionModeMenu(').replaceAll('<ModeMenu ', '<PlanModeMenu fallback={ProductionModeMenu} ')}`,
          map: null,
        };
      }
      if (file === here('./src/webview/settings/SettingsShell.tsx')) {
        const target = "from './AgentPage'";
        if (!source.includes(target)) this.error('Plan preview requires the current SettingsShell AgentPage slot');
        return { code: source.replace(target, `from '${bridge}'`), map: null };
      }
    },
  }],
  server: { port: 5204, strictPort: true },
}));
