import { homedir } from 'node:os';
import * as vscode from 'vscode';
import { AXES } from '@acpira/shared/appearance';
import { resolveLocale, type Language, type Locale } from '@acpira/shared/i18n';
import { isSharedSetting, SETTING_KEYS } from '@acpira/shared/settings';
import { normalizePreviewUrl } from '@acpira/shared/picker';
import type { BrowserOpened, PlatformEvent, PlatformMethod, PlatformRequest } from '@acpira/shared/sidecar';
import type { SecretVault } from './accounts/AccountStore';
import { WorkspaceFiles } from './files';
import type { HelloPayload } from '@acpira/engine-client/SidecarClient';
import { SharedSettingsFile } from '@acpira/engine-client/sharedSettings';
import { acpiraHome } from './store/dataDir';
import { terminalLaunch } from './terminalLaunch';
import { legacyAgents } from './legacyAgents';

// Every IDE action the sidecar may ask for; VS Code implements them all
const CAPABILITIES: PlatformMethod[] = ['openResolvedFile', 'openPlanDocument', 'revealInOS', 'searchFiles', 'writeSetting', 'openExternal', 'openBrowser', 'openInEditor', 'runInTerminal', 'toast', 'notify'];

// The view type of the chat opened as an editor tab (extension.ts creates and restores those panels)
export const EDITOR_VIEW_TYPE = 'acpira.editor';

// A group whose visible tab is the chat panel. TabInputWebview.viewType carries an internal prefix
// ("mainThreadWebview-acpira.editor"), hence the suffix match
function showsChat(g: vscode.TabGroup): boolean {
  const input = g.activeTab?.input;
  return input instanceof vscode.TabInputWebview && input.viewType.endsWith(EDITOR_VIEW_TYPE);
}

// Where files and plans opened from the chat land: as a tab in the editor group already in use, never a fresh split.
// ViewColumn.Beside split the active group every time (with the chat in a sidebar, the code group itself got halved).
// With the chat in a sidebar the active group is the code group; with the chat as an editor tab, the first other
// group (preferring one that holds tabs) takes the file, and only a lone chat group opens a split beside itself
export function fileColumn(): vscode.ViewColumn {
  const { activeTabGroup, all } = vscode.window.tabGroups;
  if (!showsChat(activeTabGroup)) return activeTabGroup.viewColumn;
  const other = all.find(g => !showsChat(g) && g.tabs.length > 0) ?? all.find(g => !showsChat(g));
  return other?.viewColumn ?? vscode.ViewColumn.Beside;
}

// The integrated browser (VS Code 1.10x+, a Chromium view) and the Simple Browser extension it superseded. Both open in
// the active editor group whatever column they are given (the Simple Browser forwards to the integrated one when that
// exists), so the target group is focused first
const INTEGRATED_BROWSER = 'workbench.action.browser.open';
const SIMPLE_BROWSER = 'simpleBrowser.api.open';
const FOCUS_GROUP = ['First', 'Second', 'Third', 'Fourth', 'Fifth', 'Sixth', 'Seventh', 'Eighth'].map(n => `workbench.action.focus${n}EditorGroup`);

// Whether the page answers from the user's machine: the chat webview renders there even when the extension host is remote
export type UrlProbe = (url: string) => Promise<{ ok: boolean; error?: string } | undefined>;

// Pages on loopback are what a remote workspace has to forward, and the only ones the chat webview's CSP lets it probe
function isLoopback(uri: vscode.Uri): boolean {
  const host = uri.authority.replace(/^.*@/, '').replace(/:\d+$/, '').toLowerCase();
  return host === 'localhost' || host === '127.0.0.1';
}

// Dev servers print the any-address they bound; it is not a host the user's machine can browse to or forward
function browsable(uri: vscode.Uri): vscode.Uri {
  const m = /^(.*@)?(0\.0\.0\.0|\[::\])(:\d+)?$/.exec(uri.authority);
  return m ? uri.with({ authority: `${m[1] ?? ''}localhost${m[3] ?? ''}` }) : uri;
}

const allTabs = () => vscode.window.tabGroups.all.flatMap(g => g.tabs);

// Make a group the active one, where both browsers open: a column that is not there (Beside: a lone chat group) gets a new
// group on the chat's left
async function focusGroup(column: vscode.ViewColumn): Promise<void> {
  const focus = column > 0 ? FOCUS_GROUP[column - 1] : 'workbench.action.newGroupLeft';
  if (focus) await vscode.commands.executeCommand(focus);
}

// Pages open_browser shows, one integrated-browser tab per site (scheme, host and port): a later call for the same site
// reuses that tab instead of opening another. The integrated browser's own reuse (`reuseUrlFilter`) navigates whenever it is
// given a URL, which reloads the page, and some pages take over their app's session on every load (Blockbench's watch page
// switches the workbench to attended and the project has to be reopened). So the same URL only brings the tab to the front;
// another page of the site navigates it. Tabs are matched by identity; one the user closed (or dragged to another group,
// which VS Code reports as a new tab) is no longer known, and the call opens or reuses through the browser's own match
class BrowserPages {
  private sites = new Map<string, { tab: vscode.Tab; url: string }>();

  async show(uri: vscode.Uri): Promise<BrowserOpened['reused']> {
    const url = uri.toString(true);
    if (!(await vscode.commands.getCommands(true)).includes(INTEGRATED_BROWSER)) {
      // Older VS Code: the Simple Browser keeps one view and reloads it on every call. Its command is registered only once its
      // extension activates, so it is not in the list before the first call
      await focusGroup(fileColumn());
      await vscode.commands.executeCommand(SIMPLE_BROWSER, uri, { viewColumn: vscode.ViewColumn.Active });
      return undefined;
    }
    const site = `${uri.scheme}://${uri.authority}`;
    // The browser matches the filter's authority and path as globs against each tab's current URL, and a bare origin parses
    // with the path "/", which matches no page; "/**" matches every page of the site
    const known = this.sites.get(site);
    const tab = known && allTabs().includes(known.tab) ? known.tab : undefined;
    // A known tab is revealed in its own group (opening into another group would move it); a new one lands next to the chat
    await focusGroup(tab ? tab.group.viewColumn : fileColumn());
    if (tab && known?.url === url) {
      // Without `url` the matching tab is only revealed. With no match this would open a blank tab, hence the check above
      await vscode.commands.executeCommand(INTEGRATED_BROWSER, { reuseUrlFilter: `${site}/**` });
      return 'revealed';
    }
    const before = new Set(allTabs());
    await vscode.commands.executeCommand(INTEGRATED_BROWSER, { url, reuseUrlFilter: `${site}/**` });
    // A tab that was not there before is new; otherwise the browser navigated a tab of that site, which is now the active one
    const fresh = allTabs().find(t => !before.has(t));
    const shown = fresh ?? vscode.window.tabGroups.activeTabGroup.activeTab;
    if (shown) this.sites.set(site, { tab: shown, url });
    return fresh ? undefined : 'navigated';
  }
}

// vscode.open uses the default editor for the resource (image preview, custom editors, text); openTextDocument rejects binaries
// ("the file appears to be binary"). Lines arrive 1-based, Range is 0-based
async function openFile(path: string, line?: number): Promise<void> {
  const at = line != null ? line - 1 : undefined;
  await vscode.commands.executeCommand('vscode.open', vscode.Uri.file(path), {
    preview: true,
    viewColumn: fileColumn(),
    ...(at != null ? { selection: new vscode.Range(at, 0, at, 0) } : {}),
  });
}

// A notification; with `open` it carries a button that opens that file when clicked (dismissing it opens nothing)
function showToast(r: Extract<PlatformRequest, { method: 'toast' }>): void {
  const show = r.level === 'error' ? vscode.window.showErrorMessage : vscode.window.showInformationMessage;
  if (!r.open) { void show(r.text); return; }
  const { label, path } = r.open;
  void show(r.text, label).then(choice => {
    if (choice === label) openFile(path).catch((e: unknown) => void vscode.window.showErrorMessage(String(e)));
  });
}

// The VS Code / Cursor side of the sidecar's platform: the facts hello carries (workspace folder, display language, acpira.* settings),
// the events that refresh them, and the IDE actions platformRequests ask for. The only host-side file besides
// extension.ts, bridge.ts and files.ts that imports vscode
export class VscodePlatform {
  readonly legacy: { from: string; vault: SecretVault };
  private readonly files = new WorkspaceFiles();
  private readonly pages = new BrowserPages();
  private readonly keys: string[];
  readonly shared = new SharedSettingsFile(acpiraHome());

  constructor(
    private context: vscode.ExtensionContext,
    private openInEditor: (sessionId?: string) => void,
    // The attention check needs the chat webviews, which extension.ts owns
    private announce: (r: Extract<PlatformRequest, { method: 'notify' }>) => void,
    // Asks a chat webview on the user's machine; undefined when no chat page is up to ask
    private probe: UrlProbe,
    // Acpira's own browser panel (extension.ts showPreview), where loopback http pages go
    private preview?: (url: string, folder?: string) => BrowserOpened['reused'],
  ) {
    // Each IDE's globalStorage / SecretStorage tree is merged into ~/.acpira once (dataDir.migrateOnce), before the sidecar starts
    this.legacy = {
      from: context.globalStorageUri.fsPath,
      vault: {
        get: async k => context.secrets.get(k),
        store: async (k, v) => context.secrets.store(k, v),
        delete: async k => context.secrets.delete(k),
      },
    };
    // The snapshot covers the keys the manifest declares (the shell's own, with VS Code's defaults) plus whatever shared
    // keys settings.json still holds: the first engine on the machine seeds <ACPIRA_HOME>/settings.json from them
    const props = (context.extension.packageJSON as { contributes?: { configuration?: { properties?: Record<string, unknown> } } }).contributes?.configuration?.properties ?? {};
    const declared = Object.keys(props).filter(k => k.startsWith('acpira.')).map(k => k.slice('acpira.'.length));
    this.keys = [...new Set([...declared, ...SETTING_KEYS.filter(isSharedSetting), ...AXES.map(a => `appearance.${a.key}`)])];
  }

  // A setting as this shell sees it: shared keys from the shared file once it exists, the shell's own from VS Code
  setting(key: string): unknown {
    return this.shared.get(key, k => this.cfg().get(k));
  }

  private cfg() { return vscode.workspace.getConfiguration('acpira'); }

  cwd() { return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? homedir(); }

  locale(): Locale { return resolveLocale(this.setting('language') as Language | undefined, vscode.env.language); }

  hello(): HelloPayload {
    return {
      client: { name: vscode.env.appName, version: String((this.context.extension.packageJSON as { version?: unknown }).version ?? '0'), capabilities: CAPABILITIES, sshConnection: process.env.SSH_CONNECTION || undefined },
      env: { cwd: this.cwd(), hostLanguage: vscode.env.language },
      settings: this.snapshot(),
    };
  }

  // Object values come back as read-only proxies; a JSON round trip makes them plain data for the wire
  private snapshot(): Record<string, unknown> {
    const cfg = this.cfg();
    const out: Record<string, unknown> = {};
    for (const k of this.keys) {
      const v = k === 'agents' ? legacyAgents(cfg.get(k), vscode.env.remoteName) : cfg.get(k);
      if (v !== undefined) out[k] = JSON.parse(JSON.stringify(v)) as unknown;
    }
    return out;
  }

  // A remote workspace's loopback page gets a tunnel to the user's machine (asExternalUri; local windows get the URL back
  // unchanged), and that address must answer from the user's side before a tab opens. The integrated browser's
  // experimental remote proxy loads pages on the remote itself, so it gets the original URL and no local check
  // The browser panel's "open in VS Code's browser": the same path without the preview routing, which would only bring
  // the panel itself up again
  openInVsCodeBrowser(url: string): Promise<BrowserOpened> {
    return this.openBrowser(url, false);
  }

  // `folder`: the session folder on an SSH host; its loopback page is that host's, so only the preview proxy reaches it
  private async openBrowser(url: string, viaPreview = true, folder?: string): Promise<BrowserOpened> {
    // A loopback http page opens in Acpira's browser panel, where its elements can be picked into the chat. The engine has
    // already found it listening on its machine (relay hub), and its preview proxy reaches it from there, so a remote dev
    // server needs no forward and no probe from the user's side. https and other hosts are beyond that proxy and stay with
    // VS Code's browser
    const page = viaPreview && this.preview && normalizePreviewUrl(url);
    if (page && this.preview) {
      const reused = this.preview(page, folder);
      return { url: page, ...(reused ? { reused } : {}) };
    }
    const target = browsable(vscode.Uri.parse(url, true));
    const proxied = !!vscode.env.remoteName && vscode.workspace.getConfiguration('workbench.browser').get<boolean>('enableRemoteProxy') === true
      && (await vscode.commands.getCommands(true)).includes(INTEGRATED_BROWSER);
    const local = proxied ? target : await vscode.env.asExternalUri(target);
    if (!proxied && isLoopback(local)) {
      const shown = local.toString(true);
      const probed = await this.probe(shown);
      if (probed && !probed.ok) {
        const via = shown === url ? '' : ` (forwarded from ${url})`;
        throw new Error(`${shown}${via} did not answer from the user's machine: ${probed.error ?? 'no response'}. The port forward may have failed; the user can check the Ports view.`);
      }
    }
    const reused = await this.pages.show(local);
    return { url: local.toString(true), ...(reused ? { reused } : {}) };
  }

  // Settings edits (the settings page, settings.json, the Settings UI), window focus and workspace folder changes, as platform events
  subscribe(emit: (event: PlatformEvent) => void): vscode.Disposable {
    return vscode.Disposable.from(
      vscode.workspace.onDidChangeConfiguration(e => {
        if (!e.affectsConfiguration('acpira')) return;
        const keys = this.keys.filter(k => e.affectsConfiguration(`acpira.${k}`));
        if (keys.length) emit({ type: 'settingsChanged', keys, settings: this.snapshot() });
      }),
      vscode.window.onDidChangeWindowState(e => { if (e.focused) emit({ type: 'windowFocus' }); }),
      vscode.workspace.onDidChangeWorkspaceFolders(() => emit({ type: 'envChanged', env: { cwd: this.cwd() } })),
    );
  }

  async handle(r: PlatformRequest): Promise<unknown> {
    switch (r.method) {
      case 'writeSetting': await this.cfg().update(r.key, r.value, vscode.ConfigurationTarget.Global); return null;
      case 'searchFiles': return this.files.search(r.query);
      case 'openResolvedFile': await openFile(r.path, r.line); return null;
      case 'openPlanDocument': {
        const doc = 'path' in r.target ? await vscode.workspace.openTextDocument(vscode.Uri.file(r.target.path))
          : await vscode.workspace.openTextDocument({ language: 'markdown', content: r.target.markdown });
        await vscode.window.showTextDocument(doc, { preview: true, viewColumn: fileColumn() });
        return null;
      }
      case 'revealInOS': await vscode.commands.executeCommand('revealFileInOS', vscode.Uri.file(r.path)); return null;
      case 'openExternal': void vscode.env.openExternal(vscode.Uri.parse(r.url)); return null;
      case 'openBrowser': return this.openBrowser(r.url, true, r.folder);
      case 'openInEditor': this.openInEditor(r.sessionId); return null;
      case 'toast': showToast(r); return null;
      case 'notify': this.announce(r); return null;
      case 'runInTerminal': {
        const { text, ...launch } = terminalLaunch(r.command, r.args);
        // One-shot terminals (logins, unlocks, installs) must not be persisted: on Windows the command is the shell
        // itself, so a revived terminal re-runs it on the next window open (a stale `claude auth login` opens claude.ai)
        const t = vscode.window.createTerminal({ name: r.title, env: r.env, isTransient: true, ...launch });
        t.show();
        if (text !== undefined) t.sendText(text);
        return null;
      }
    }
  }
}
