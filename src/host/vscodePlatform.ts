import { homedir } from 'node:os';
import * as vscode from 'vscode';
import { resolveLocale, type Language, type Locale } from '@shared/i18n';
import type { PlatformEvent, PlatformMethod, PlatformRequest } from '@shared/sidecar';
import type { SecretVault } from './accounts/AccountStore';
import { WorkspaceFiles } from './files';
import type { HelloPayload } from './shell/SidecarClient';
import { terminalLaunch } from './terminalLaunch';
import { legacyAgents } from './legacyAgents';

// Every IDE action the sidecar may ask for; VS Code implements them all
const CAPABILITIES: PlatformMethod[] = ['openResolvedFile', 'openPlanDocument', 'revealInOS', 'searchFiles', 'writeSetting', 'openExternal', 'openInEditor', 'runInTerminal', 'toast'];

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
function fileColumn(): vscode.ViewColumn {
  const { activeTabGroup, all } = vscode.window.tabGroups;
  if (!showsChat(activeTabGroup)) return activeTabGroup.viewColumn;
  const other = all.find(g => !showsChat(g) && g.tabs.length > 0) ?? all.find(g => !showsChat(g));
  return other?.viewColumn ?? vscode.ViewColumn.Beside;
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
  private readonly keys: string[];

  constructor(private context: vscode.ExtensionContext, private openInEditor: (sessionId?: string) => void) {
    // Each IDE's globalStorage / SecretStorage tree is merged into ~/.acpira once (dataDir.migrateOnce), before the sidecar starts
    this.legacy = {
      from: context.globalStorageUri.fsPath,
      vault: {
        get: async k => context.secrets.get(k),
        store: async (k, v) => context.secrets.store(k, v),
        delete: async k => context.secrets.delete(k),
      },
    };
    // The settings snapshot covers every acpira.* key the manifest declares, so readers see VS Code's defaults like before
    const props = (context.extension.packageJSON as { contributes?: { configuration?: { properties?: Record<string, unknown> } } }).contributes?.configuration?.properties ?? {};
    this.keys = Object.keys(props).filter(k => k.startsWith('acpira.')).map(k => k.slice('acpira.'.length));
  }

  private cfg() { return vscode.workspace.getConfiguration('acpira'); }

  cwd() { return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? homedir(); }

  locale(): Locale { return resolveLocale(this.cfg().get<Language>('language'), vscode.env.language); }

  hello(): HelloPayload {
    return {
      client: { name: vscode.env.appName, version: String((this.context.extension.packageJSON as { version?: unknown }).version ?? '0'), capabilities: CAPABILITIES },
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
      case 'openInEditor': this.openInEditor(r.sessionId); return null;
      case 'toast': showToast(r); return null;
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
