import { useSyncExternalStore } from 'react';
import type { BrowserHostMsg, BrowserMsg, BrowserPage } from '@acpira/shared/protocol';
import { vscodeApi, type VsCodeApi } from '../vscodeApi';
import { panel } from './store';

// The tool panel's browser tabs run the browser page (host 'browser', webview/browser/BrowserPanel.tsx) in a frame of this
// page's own origin, one per tab. A frame gets no preload of its own: its page takes its transport from here
// (rust/crates/acpira-desktop/src/page.rs sets `window.__acpiraApi = parent.__acpiraBrowserTab(tab)` before the bundle runs). Its messages go
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
  // The frames follow this page's theme class (the system appearance, apps/desktop/src/init.js): no preload runs in them
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
export const MAX_RECENT_PAGES = 5;

function loopbackHost(raw: string): string {
  try {
    const u = new URL(raw);
    const h = u.hostname === '127.0.0.1' || u.hostname === '[::1]' ? 'localhost' : u.hostname;
    return u.port ? `${h}:${u.port}` : h;
  } catch {
    return raw;
  }
}

function pageKey(page: BrowserPage): string {
  const host = loopbackHost(page.url);
  const title = page.title.trim();
  if (title) return `${host}\n${title}`;
  try { return `${host}\n${new URL(page.url).pathname}`; } catch { return page.url; }
}

// Collapses query-string or loopback-host variants that share the same title and port, keeping at most `limit` pages
// (preferring a live URL when one of the duplicates is currently answering)
export function dedupeRecentPages(pages: readonly BrowserPage[], running: ReadonlySet<string>, limit = MAX_RECENT_PAGES): BrowserPage[] {
  const byKey = new Map<string, BrowserPage>();
  for (const page of pages) {
    const k = pageKey(page);
    const prev = byKey.get(k);
    if (!prev) {
      if (byKey.size < limit) byKey.set(k, page);
    } else if (!running.has(prev.url) && running.has(page.url)) {
      byKey.set(k, page);
    }
  }
  return [...byKey.values()];
}

let recent: RecentPages = { pages: [], running: [] };
const recentListeners = new Set<() => void>();

export function setRecentPages(next: RecentPages) {
  recent = next;
  for (const l of recentListeners) l();
}

export function useRecentPages(): RecentPages {
  return useSyncExternalStore(l => { recentListeners.add(l); return () => { recentListeners.delete(l); }; }, () => recent);
}
