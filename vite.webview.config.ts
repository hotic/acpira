import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';

// Webview build: root is src/webview.
// Only index.html is built: main.js and main.css under fixed names for the bridges, plus the chunks the page imports on
// demand (mermaid, the highlighting worker, img-fx …) under assets/, which every host serves from the same directory.
// A chunk loads through `import()` from main.js and inherits its CSP nonce. All CSS stays in main.css, and Vite's
// modulepreload helper is off: its injected <link> tags would carry no nonce
const root = fileURLToPath(new URL('./src/webview', import.meta.url));

export default defineConfig({
  root,
  base: './',
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@shared': fileURLToPath(new URL('./src/shared', import.meta.url)),
      '@webview': root,
    },
  },
  build: {
    outDir: fileURLToPath(new URL('./dist/webview', import.meta.url)),
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
