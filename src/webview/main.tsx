// First: its request starts before the rest of the bundle evaluates
import './fontPreload';
import { createRoot } from 'react-dom/client';
import '@fontsource-variable/inter';
import '@fontsource-variable/geist';
import 'katex/dist/katex.min.css';
import './styles/index.css';
import { App } from './App';
import { BrowserPanel } from './browser/BrowserPanel';

// The browser panel runs the same bundle in its own webview (src/host/bridge.ts BrowserPanel)
createRoot(document.getElementById('root')!).render(window.__acpira?.host === 'browser' ? <BrowserPanel /> : <App />);
