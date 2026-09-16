import { stat } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { isSafeExternalUrl, type FileHit, type HostMsg, type WebviewHost, type WebviewMsg } from '@shared/protocol';
import type { Appearance } from '@shared/appearance';
import type { SessionManager, SessionViewer } from './SessionManager';
import type { SettingsCenter } from './settings';
import type { HostPlatform } from './platform';
import { msg } from './errors';
import { freezeHostMsg, HostMsgBatch } from './msgBatch';

export interface BridgeCoreDeps {
  manager: SessionManager;
  settings: SettingsCenter;
  platform: HostPlatform;
  appearance: () => Appearance;
}

export interface BridgeCoreOpts {
  // Where this view's HostMsgs go (the webview's postMessage, or the sidecar envelope for a viewId)
  post: (m: HostMsg) => void;
  host: WebviewHost;
  // How the view loads attachment blobs: `${blobBase}/${sessionId}/${blob}` (a webview URI of the sessions dir, or the shell's resource host)
  blobBase?: string;
  // The session it opens on: an id, the most recent session, or nothing → a fresh session
  initial?: string | { mostRecent: true };
}

// One per view (a VS Code webview, a JCEF browser): routes WebviewMsgs to its viewer / the manager / the settings center, resolves
// paths and makes the business decisions before asking the platform for an IDE action, and pushes changes back after coalescing.
// Nothing here knows how the view is rendered or transported
export class BridgeCore {
  readonly viewer: SessionViewer;
  private ready = false;
  private readonly batch = new HostMsgBatch(m => this.post(m));
  private readonly unsubscribe: (() => void)[] = [];

  constructor(private deps: BridgeCoreDeps, private opts: BridgeCoreOpts) {
    this.viewer = deps.manager.attach(opts.initial);
    this.unsubscribe.push(this.viewer.subscribe(ev => this.queue(ev)), deps.settings.subscribe(ev => this.queue(ev)));
  }

  // Failures are logged, never thrown at the transport: a bad message must not take the view down
  async handle(m: WebviewMsg): Promise<void> {
    try {
      await this.route(m);
    } catch (e) {
      this.deps.platform.log(`webview ${m.type} failed: ${msg(e)}`);
    }
  }

  private async route(m: WebviewMsg) {
    const { manager, platform } = this.deps;
    if (m.type === 'ready') {
      this.ready = true;
      // A view (re)opening is a cheap moment to re-check the executables (a change arrives as an `agents` event after init) and to pick up
      // sessions another window created since; the list is reconciled before ensureActive so a sidebar starting on "most recent" sees them
      void manager.reprobe();
      await manager.refreshIndex();
      await this.viewer.ensureActive();
      this.post({
        type: 'init',
        state: {
          host: this.opts.host, appearance: this.deps.appearance(), agents: manager.agents(), accounts: manager.accounts(), accountActions: manager.accountActions(), hidden: manager.hidden(),
          sessions: manager.sessions(), active: this.viewer.active(), blobBase: this.opts.blobBase,
          settings: this.deps.settings.view(), locale: this.deps.settings.locale(), home: platform.home(), cwd: platform.cwd(),
        },
      });
      return;
    }
    if (m.type === 'chatgptStatus') { this.post({ type: 'chatgptStatus', status: await manager.chatgptStatus() }); return; }
    if (m.type === 'openInEditor') { platform.openInEditor(m.sessionId ?? this.viewer.activeId); return; }
    if (m.type === 'openFile') {
      // Tool references are relative to the session they came from; a view showing another session ignores them
      const session = this.viewer.active();
      if (!session || session.id !== m.sessionId) return;
      try {
        const path = m.path.startsWith('file://') ? fileURLToPath(m.path) : resolve(session.cwd, m.path);
        const line = Number.isSafeInteger(m.line) && m.line! > 0 ? m.line : undefined;
        await platform.openResolvedFile(path, line);
      } catch (e) {
        platform.toast('error', msg(e));
      }
      return;
    }
    if (m.type === 'openPlan') {
      const plan = manager.planDocument(m.sessionId, m.planId);
      if (plan?.type === 'plan_document') {
        const exists = plan.path && await stat(plan.path).catch(() => undefined);
        await platform.openPlanDocument(exists && plan.path ? { path: plan.path } : { markdown: plan.markdown });
      }
      return;
    }
    if (m.type === 'openExternal') {
      if (isSafeExternalUrl(m.url)) platform.openExternal(m.url);
      else platform.log(`openExternal refused: scheme not on the allowlist (${m.url.slice(0, 80)})`);
      return;
    }
    if (m.type === 'searchFiles') {
      // Always answer, even on failure: the webview holds a promise per seq
      let files: FileHit[] = [];
      try { files = await platform.searchFiles(m.query); } catch (e) { platform.log(`searchFiles failed: ${msg(e)}`); }
      this.post({ type: 'files', seq: m.seq, files });
      return;
    }
    if (m.type === 'editTurn') {
      try {
        await manager.editTurn(m.edit);
        this.post({ type: 'editTurnResult', requestId: m.requestId });
      } catch (e) {
        this.post({ type: 'editTurnResult', requestId: m.requestId, error: msg(e) });
      }
      return;
    }
    if (await this.onSettingsMessage(m)) return;
    await this.viewer.handle(m);
  }

  // The settings page's requests; returns true when the message was its business
  private async onSettingsMessage(m: WebviewMsg): Promise<boolean> {
    const { manager, settings, platform } = this.deps;
    try {
      switch (m.type) {
        case 'setSetting': await settings.set(m.key, m.value); return true;
        case 'setAppearance': await settings.setAppearance(m.axis, m.value); return true;
        // A path from the inventory lists: files open in the editor, directories reveal in the OS file manager
        case 'openPath': {
          const s = await stat(m.path).catch(() => undefined);
          if (s?.isDirectory()) await platform.revealInOS(m.path); else await platform.openResolvedFile(m.path);
          return true;
        }
        case 'inventory': this.post({ type: 'inventory', agent: m.agent, inventory: await settings.inventory(m.agent) }); return true;
        case 'controls': {
          if (!m.fresh) { this.post({ type: 'controls', agent: m.agent, controls: await manager.knownControls(m.agent) }); return true; }
          // The probe's initialize brings the version back too; a fresh inventory after the list keeps the facts card in step
          this.post({ type: 'controls', agent: m.agent, controls: await manager.probeControls(m.agent) });
          this.post({ type: 'inventory', agent: m.agent, inventory: await settings.inventory(m.agent) });
          return true;
        }
        default: return false;
      }
    } catch (e) {
      platform.log(`settings ${m.type} failed: ${msg(e)}`);
      return true;
    }
  }

  // Streaming updates are dense; for the same message type within one batch window, keep only the latest
  private queue(m: HostMsg) {
    if (!this.ready) return;
    this.batch.push(m);
  }

  post(m: HostMsg) { this.opts.post(freezeHostMsg(m)); }

  pushAppearance() { this.post({ type: 'appearance', appearance: this.deps.appearance() }); }

  dispose() {
    this.batch.dispose();
    for (const u of this.unsubscribe) u();
    this.viewer.dispose();
  }
}
