import { useSyncExternalStore } from 'react';
import type { BrowserHostMsg, BrowserMsg, BrowserPage } from '@shared/protocol';
import { vscodeApi, type VsCodeApi } from '../vscodeApi';
import { panel } from './store';

// The tool panel's browser tabs run the browser page (host 'browser', webview/browser/BrowserPanel.tsx) in a frame of this
// page's own origin, one per tab. A frame gets no preload of its own: its page takes its transport from here
// (apps/desktop/src/page.ts sets `window.__acpiraApi = parent.__acpiraBrowserTab(tab)` before the bundle runs). Its messages go
// up tagged with the tab (`panelBrowser`), the shell's answers come back the same way and are posted into that frame. The
// pages the browser shows live in frames inside those, on loopback origins, which cannot reach this window

declare global {
  interface Window { __acpiraBrowserTab?: (tab: string) => VsCodeApi }
}

const frames = new Map<string, HTMLIFrameElement>();

export function installBrowserTabs() {
  if (typeof window === 'undefined' || window.__acpiraBrowserTab) return;
  window.__acpiraBrowserTab = (tab: string): VsCodeApi => ({
    postMessage(raw: unknown) {
      const msg = raw as BrowserMsg;
      // The tab is named after its page
      if (msg.type === 'browserTitle') panel.titled(tab, msg.url, msg.title);
      vscodeApi().postMessage({ type: 'panelBrowser', tab, msg });
    },
    // A tab's page keeps nothing across reloads: the shell resends its frame when the page says it is ready
    getState: () => undefined,
    setState: () => {},
  });
  // The frames follow this page's theme class (the system appearance, apps/desktop/src/preload.ts): no preload runs in them
  new MutationObserver(() => { for (const frame of frames.values()) syncTheme(frame); })
    .observe(document.body, { attributes: true, attributeFilter: ['class'] });
}

function syncTheme(frame: HTMLIFrameElement) {
  const body = frame.contentDocument?.body;
  if (body && body.className !== document.body.className) body.className = document.body.className;
}

export function registerBrowserFrame(tab: string, frame: HTMLIFrameElement | null) {
  if (!frame) { frames.delete(tab); return; }
  frames.set(tab, frame);
  frame.addEventListener('load', () => syncTheme(frame));
}

// The shell's message for one tab's page; the page posted browserReady before the shell sends anything, so it listens
export function deliverToBrowserTab(tab: string, msg: BrowserHostMsg) {
  frames.get(tab)?.contentWindow?.postMessage(msg, location.origin);
}

// The new-tab page's recent pages, as the shell last answered panelHistory
export interface RecentPages { pages: BrowserPage[]; running: string[] }
let recent: RecentPages = { pages: [], running: [] };
const recentListeners = new Set<() => void>();

export function setRecentPages(next: RecentPages) {
  recent = next;
  for (const l of recentListeners) l();
}

export function useRecentPages(): RecentPages {
  return useSyncExternalStore(l => { recentListeners.add(l); return () => { recentListeners.delete(l); }; }, () => recent);
}
