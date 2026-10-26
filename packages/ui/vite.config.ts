import { defineConfig, type Plugin } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

// Webview build: root is this directory (packages/ui), the output the repository's dist/webview, which every shell serves.
// Only index.html is built: main.js and main.css under fixed names for the bridges, plus the chunks the page imports on
// demand (mermaid, the highlighting worker, img-fx …) under assets/, which every host serves from the same directory.
// A chunk loads through `import()` from main.js and inherits its CSP nonce. All CSS stays in main.css, and Vite's
// modulepreload helper is off: its injected <link> tags would carry no nonce
const root = dirname(fileURLToPath(import.meta.url));

// katex.min.css lists woff2, woff and ttf for every face. Every host webview (VS Code's Electron, JCEF, WKWebView) loads
// woff2, so the two fallbacks are only dead weight in every package (~0.9 MB); with their url()s gone they are not emitted
const katexWoff2Only: Plugin = {
  name: 'acpira:katex-woff2-only',
  enforce: 'pre',
  transform(code, id) {
    if (!/[\\/]katex[\\/]dist[\\/]katex(\.min)?\.css$/.test(id)) return;
    return code.replace(/,url\([^)]+\.(?:woff|ttf)\) format\("(?:woff|truetype)"\)/g, '');
  },
};

export default defineConfig({
  root,
  base: './',
  plugins: [katexWoff2Only, react(), tailwindcss()],
  build: {
    outDir: fileURLToPath(new URL('../../dist/webview', import.meta.url)),
    emptyOutDir: true,
    sourcemap: false,
    cssCodeSplit: false,
    modulePreload: false,
    rollupOptions: {
      input: { main: `${root}/index.html` },
      output: {
        entryFileNames: 'main.js',
        chunkFileNames: 'assets/[name]-[hash].js',
        assetFileNames: a => (a.names?.some(n => n.endsWith('.css')) ? 'main.css' : 'assets/[name][extname]'),
      },
    },
  },
});
