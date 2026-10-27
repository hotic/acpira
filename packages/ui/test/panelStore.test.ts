import { describe, expect, it } from 'vitest';
import { dedupeRecentPages } from '../panel/browserTabs';
import { FILES_TAB, anyModule, panel, panelState } from '../panel/store';
import { setPluginSlots } from '../pluginSlots';

describe('dedupeRecentPages', () => {
  it('collapses repeated titles on the same loopback port, prefers running URLs, and caps at 5', () => {
    const pages = [
      { url: 'http://localhost:5199/lab/tour.preview.html?s=1', title: '设置里的界面导览入口' },
      { url: 'http://localhost:5199/lab/first.preview.html?v=1', title: '首次启动：导览与数据目录' },
      { url: 'http://localhost:5199/lab/first.preview.html?v=2', title: '首次启动：导览与数据目录' },
      { url: 'http://127.0.0.1:5199/lab/lightbox.preview.html?v=1', title: 'Lightbox 预览选型' },
      { url: 'http://localhost:5199/lab/lightbox.preview.html?v=2', title: 'Lightbox 预览选型' },
      { url: 'http://localhost:3000/', title: 'App A' },
      { url: 'http://localhost:3001/', title: 'App B' },
      { url: 'http://localhost:3002/', title: 'App C' },
    ];
    const running = new Set(['http://localhost:5199/lab/lightbox.preview.html?v=2']);
    const out = dedupeRecentPages(pages, running);
    expect(out).toHaveLength(5);
    expect(out.map(p => p.title)).toEqual([
      '设置里的界面导览入口',
      '首次启动：导览与数据目录',
      'Lightbox 预览选型',
      'App A',
      'App B',
    ]);
    // The running duplicate takes over the dead URL for the same title and port
    expect(out[2]!.url).toBe('http://localhost:5199/lab/lightbox.preview.html?v=2');
  });
});

describe('panel.setFiles hot-plugging', () => {
  it('removes the Files tab when switched off, keeps terminal and browser tabs, and restores Files when switched back on', () => {
    panel.setFiles(true);
    panel.setContext('s1', '/proj');
    expect(panelState().tabs.map(t => t.kind)).toEqual(['files']);

    // Unplugging Files with no other tabs open leaves a single new-tab launcher
    panel.setFiles(false);
    expect(panelState().files).toBe(false);
    expect(panelState().tabs).toHaveLength(1);
    expect(panelState().tabs[0]!.kind).toBe('new');

    // Opening a terminal replaces the launcher; closing the last terminal returns to a new-tab launcher
    const termId = panel.openTerminal('/proj', panelState().tabs[0]!.id);
    expect(panelState().tabs.map(t => t.kind)).toEqual(['terminal']);
    const browserId = panel.openBrowser('http://localhost:3000/');
    expect(panelState().tabs.map(t => t.kind)).toEqual(['terminal', 'browser']);

    // Plugging Files back in restores the permanent Files tab at the front while keeping terminal and browser tabs
    panel.setFiles(true);
    expect(panelState().tabs.map(t => t.id)).toEqual([FILES_TAB, termId, browserId]);

    // Unplugging again drops Files and keeps terminal and browser; closing both returns to the new-tab launcher
    panel.setFiles(false);
    expect(panelState().tabs.map(t => t.id)).toEqual([termId, browserId]);
    panel.close(termId);
    panel.close(browserId);
    expect(panelState().tabs).toHaveLength(1);
    expect(panelState().tabs[0]!.kind).toBe('new');

    // Unplugging all three modules closes the panel and prevents toggle from opening it
    panel.toggle(true);
    expect(panelState().open).toBe(true);
    panel.setModules({ files: false, terminal: false, browser: false });
    expect(panelState().open).toBe(false);
    panel.toggle(true);
    expect(panelState().open).toBe(false);

    // Restore default state
    panel.setModules({ files: true, terminal: true, browser: true });
  });
});

describe('plugin tab pieces', () => {
  const piece = (id: string, title = id) => ({ plugin: 'notes', id, slot: 'panel.tab' as const, title, html: `<p>${id}</p>` });

  it('open as tabs of their own, take the tab again when asked twice, and close when their plugin goes', () => {
    panel.setModules({ files: true, terminal: true, browser: true });
    panel.setContext('s-plugin', '/proj-plugin');
    setPluginSlots([piece('board', 'Board')]);
    const id = panel.openPlugin('notes', 'board', 'Board');
    expect(panelState().tabs.find(t => t.id === id)).toMatchObject({ kind: 'plugin', plugin: 'notes', piece: 'board', title: 'Board' });
    // The same piece again (from the new-tab page) takes the tab and drops the new-tab page it was picked on
    const page = panel.newTab();
    expect(panel.openPlugin('notes', 'board', 'Board', page)).toBe(id);
    expect(panelState().tabs.map(t => t.id)).toEqual([FILES_TAB, id]);
    expect(panelState().tab).toBe(id);
    // A new title follows; the piece gone (switched off, safe mode) takes its tab away
    setPluginSlots([piece('board', 'Kanban')]);
    panel.syncPlugins();
    expect(panelState().tabs.find(t => t.id === id)).toMatchObject({ title: 'Kanban' });
    setPluginSlots([]);
    panel.syncPlugins();
    expect(panelState().tabs.map(t => t.kind)).toEqual(['files']);
    expect(panelState().tab).toBe(FILES_TAB);
  });

  it('keep the panel usable with every built-in module switched off', () => {
    panel.setContext('s-plugin-2', '/proj-plugin-2');
    setPluginSlots([piece('board')]);
    panel.setModules({ files: false, terminal: false, browser: false });
    expect(anyModule(panelState())).toBe(true);
    panel.toggle(true);
    expect(panelState().open).toBe(true);
    panel.openPlugin('notes', 'board', 'board', panelState().tabs[0]!.id);
    expect(panelState().tabs.map(t => t.kind)).toEqual(['plugin']);
    setPluginSlots([]);
    panel.syncPlugins();
    expect(anyModule(panelState())).toBe(false);
    expect(panelState().open).toBe(false);
    panel.setModules({ files: true, terminal: true, browser: true });
  });
});
