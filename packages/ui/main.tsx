// First: its request starts before the rest of the bundle evaluates
import './fontPreload';
import { createRoot } from 'react-dom/client';
import '@fontsource-variable/inter';
import '@fontsource-variable/geist';
import 'katex/dist/katex.min.css';
import './styles/index.css';
import { App, bootReady } from './App';
import { BrowserPanel } from './browser/BrowserPanel';
import { fixSpacedPinyinCommits } from './ui/ime';

fixSpacedPinyinCommits();

// The browser panel runs the same bundle in its own webview (apps/vscode/src/bridge.ts BrowserPanel)
// After the boot language's dictionary (App.tsx bootReady), read from the package in a few ms
void bootReady.then(() => createRoot(document.getElementById('root')!).render(window.__acpira?.host === 'browser' ? <BrowserPanel /> : <App />));
