import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { describe, expect, it } from 'vitest';
import { DEFAULT_VIEW, readView, type ListView } from '../packages/ui/chat/sessionView';

const script = readFileSync(new URL('../apps/desktop/src/init.js', import.meta.url), 'utf8')
  .replace('__ACPIRA_APP_ORIGIN__', 'acpira-app://localhost');
const STATE_KEY = 'acpira.webviewState';
const VIEW_KEY = 'acpira.sessionListView';
type StorageApi = Pick<Storage, 'getItem' | 'setItem'>;
const storage = (): StorageApi => {
  const values = new Map<string, string>();
  return { getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); } };
};

// Run the real injected transport with fresh window storage, sharing only persistent storage across launches.
function launch(localStorage: StorageApi, sessionStorage = storage(), profile = '') {
  const window = {
    top: undefined as unknown,
    __TAURI_INTERNALS__: { invoke: async () => {}, transformCallback: () => 1 },
    __acpiraApi: undefined as { getState(): Record<string, unknown> | undefined; setState(state: unknown): void } | undefined,
  };
  window.top = window;
  runInNewContext(script.replace("'__ACPIRA_STATE_SCOPE__'", JSON.stringify(profile)), {
    window, localStorage, sessionStorage, TextEncoder,
    location: { protocol: 'acpira-app:', host: 'localhost' },
    navigator: { platform: 'MacIntel' },
    document: { addEventListener() {} },
    addEventListener() {},
  });
  return window.__acpiraApi!;
}

const view: ListView = {
  ...DEFAULT_VIEW, group: 'agent', order: 'title', per: 10, model: true, pinned: 'project',
  status: ['unread', 'background'], agents: ['codex'], since: 'week',
};

describe('desktop list preferences', () => {
  it('restores every list option after closing a window without restoring its other state', () => {
    const disk = storage();
    launch(disk).setState({ sessionListView: view, acpiraSessionId: 'old-session', drafts: { text: 'private draft' }, toolPanel: { open: true } });
    expect(launch(disk).getState()).toEqual({ sessionListView: view });
    expect(readView(launch(disk).getState()?.sessionListView)).toEqual(view);
  });

  it('keeps an existing window choice when another window saves different preferences', () => {
    const disk = storage();
    const first = launch(disk);
    first.setState({ sessionListView: view });
    const second = launch(disk);
    const changed = { ...view, pinned: 'top', per: 0 };
    second.setState({ ...second.getState(), sessionListView: changed });
    expect(first.getState()?.sessionListView).toEqual(view);
    expect(second.getState()?.sessionListView).toEqual(changed);
    expect(launch(disk).getState()?.sessionListView).toEqual(changed);
  });

  it('retains and saves preferences from an older window with only session storage', () => {
    const disk = storage();
    const session = storage();
    session.setItem(STATE_KEY, JSON.stringify({ sessionListView: view, acpiraSessionId: 'current-session' }));
    const api = launch(disk, session);
    expect(api.getState()?.sessionListView).toEqual(view);
    api.setState({ ...api.getState(), desktopColumnHidden: true });
    expect(launch(disk).getState()).toEqual({ sessionListView: view });
  });

  it('does not let another window draft overwrite the last explicit list choice', () => {
    const disk = storage();
    const first = launch(disk);
    first.setState({ sessionListView: view });
    const changed = { ...view, per: 0, pinned: 'top' };
    launch(disk).setState({ sessionListView: changed });
    first.setState({ ...first.getState(), drafts: { text: 'a new draft' } });
    expect(first.getState()?.sessionListView).toEqual(view);
    expect(launch(disk).getState()?.sessionListView).toEqual(changed);
  });

  it('does not discard preferences when a page writes state without list options', () => {
    const disk = storage();
    launch(disk).setState({ sessionListView: view });
    launch(disk).setState({ acpiraBrowserUrl: 'http://localhost/' });
    expect(launch(disk).getState()).toEqual({ sessionListView: view });
  });

  it('isolates custom desktop profiles even when WebKit shares persistent storage', () => {
    const disk = storage();
    launch(disk).setState({ sessionListView: view });
    const profile = '/test/profile-"one"';
    const custom = launch(disk, storage(), profile);
    expect(custom.getState()).toBeUndefined();
    const changed = { ...view, per: 0, pinned: 'top' };
    custom.setState({ sessionListView: changed });
    expect(launch(disk, storage(), profile).getState()).toEqual({ sessionListView: changed });
    expect(launch(disk).getState()).toEqual({ sessionListView: view });
  });

  it('recovers from malformed storage and still saves window state if persistent storage is unavailable', () => {
    const disk = storage();
    disk.setItem(VIEW_KEY, '{');
    expect(launch(disk).getState()).toBeUndefined();
    disk.setItem(VIEW_KEY, JSON.stringify(view));
    const session = storage();
    session.setItem(STATE_KEY, '{');
    expect(launch(disk, session).getState()).toEqual({ sessionListView: view });

    const unavailable: StorageApi = { getItem() { throw new Error('unavailable'); }, setItem() { throw new Error('quota'); } };
    const api = launch(unavailable);
    expect(api.getState()).toBeUndefined();
    api.setState({ sessionListView: view, acpiraSessionId: 'current-session' });
    expect(api.getState()).toEqual({ sessionListView: view, acpiraSessionId: 'current-session' });
  });
});
