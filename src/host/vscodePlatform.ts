import { homedir } from 'node:os';
import * as vscode from 'vscode';
import { resolveLocale, type Language, type Locale } from '@shared/i18n';
import type { PlatformEvent, PlatformMethod, PlatformRequest } from '@shared/sidecar';
import type { SecretVault } from './accounts/AccountStore';
import { WorkspaceFiles } from './files';
import type { HelloPayload } from './shell/SidecarClient';

// Every IDE action the sidecar may ask for; VS Code implements them all
const CAPABILITIES: PlatformMethod[] = ['openResolvedFile', 'openPlanDocument', 'revealInOS', 'searchFiles', 'writeSetting', 'openExternal', 'openInEditor', 'runInTerminal', 'toast'];

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
      const v = cfg.get(k);
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
      case 'openResolvedFile': {
        // vscode.open uses the default editor for the resource (image preview, custom editors, text); openTextDocument rejects binaries
        // ("the file appears to be binary"). Lines arrive 1-based, Range is 0-based
        const at = r.line != null ? r.line - 1 : undefined;
        await vscode.commands.executeCommand('vscode.open', vscode.Uri.file(r.path), {
          preview: true,
          viewColumn: vscode.ViewColumn.Beside,
          ...(at != null ? { selection: new vscode.Range(at, 0, at, 0) } : {}),
        });
        return null;
      }
      case 'openPlanDocument': {
        const doc = 'path' in r.target ? await vscode.workspace.openTextDocument(vscode.Uri.file(r.target.path))
          : await vscode.workspace.openTextDocument({ language: 'markdown', content: r.target.markdown });
        await vscode.window.showTextDocument(doc, { preview: true, viewColumn: vscode.ViewColumn.Beside });
        return null;
      }
      case 'revealInOS': await vscode.commands.executeCommand('revealFileInOS', vscode.Uri.file(r.path)); return null;
      case 'openExternal': void vscode.env.openExternal(vscode.Uri.parse(r.url)); return null;
      case 'openInEditor': this.openInEditor(r.sessionId); return null;
      case 'toast': void (r.level === 'error' ? vscode.window.showErrorMessage(r.text) : vscode.window.showInformationMessage(r.text)); return null;
      case 'runInTerminal': {
        const t = vscode.window.createTerminal({ name: r.title, env: r.env });
        t.show();
        t.sendText([r.command, ...r.args].map(shellQuote).join(' '));
        return null;
      }
    }
  }
}

// Commands run in a terminal: paths with spaces (/Applications/Devin.app/…) need quoting
function shellQuote(s: string): string {
  return /^[\w./=:@%+-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`;
}
