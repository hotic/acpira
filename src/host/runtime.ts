import { join } from 'node:path';
import { appearanceFromSettings, type Appearance, type AxisKey } from '@shared/appearance';
import { SETTING_KEYS, sanitizeSetting, type HiddenMap } from '@shared/settings';
import type { WebviewHost } from '@shared/protocol';
import { AgentRegistry, type CustomAgentSetting } from './acp/AgentRegistry';
import { AccountManager } from './accounts/AccountManager';
import { AccountStore, FileVault } from './accounts/AccountStore';
import { DevinAccountProvider } from './accounts/devin';
import { LocalAccounts } from './accounts/local';
import { BridgeCore, type BridgeCoreOpts } from './bridgeCore';
import { msg } from './errors';
import { setHostLocale, t } from './i18n';
import type { HostPlatform, SettingsAffects } from './platform';
import { SessionManager } from './SessionManager';
import { SettingsCenter } from './settings';
import { acpiraHome, migrateOnce } from './store/dataDir';
import { TranscriptStore } from './store/TranscriptStore';

export interface HostRuntimeOpts {
  // ACPIRA_HOME / ~/.acpira by default; tests point it at a temp directory
  home?: string;
}

// The one composition root: every host (the VS Code extension, the sidecar) builds the same registry, vault, account layer, session
// manager and settings center from a HostPlatform here, and reacts to its settings / focus events the same way. Views attach as
// BridgeCores and are torn down with the runtime
export class HostRuntime {
  readonly manager: SessionManager;
  readonly settings: SettingsCenter;
  readonly sessionsDir: string;
  private bridges = new Set<BridgeCore>();
  private activeRegistry: AgentRegistry;
  private unsubscribe: (() => void)[] = [];

  private constructor(private platform: HostPlatform, root: string, vault: FileVault, accountStore: AccountStore) {
    const log = (line: string) => platform.log(line);
    const registry = () => new AgentRegistry(this.read<Record<string, CustomAgentSetting>>('agents') ?? {});
    const runInTerminal = platform.runInTerminal.bind(platform);
    const toast = platform.toast.bind(platform);
    this.activeRegistry = registry();
    const accounts = new AccountManager({
      store: accountStore,
      providers: [new DevinAccountProvider(join(root, 'scratch'), () => this.activeRegistry.resolveBinary('devin'))],
      log, runInTerminal, toast,
    });

    this.sessionsDir = join(root, 'sessions');
    this.manager = new SessionManager({
      registry: this.activeRegistry,
      store: new TranscriptStore(this.sessionsDir, log, { onSaveError: (_id, error) => toast('error', t('host.saveFailed', { error })) }),
      log,
      cwd: () => platform.cwd(),
      defaultAgent: () => this.read<string>('defaultAgent') ?? 'grok',
      runInTerminal, toast, accounts,
      localAccounts: new LocalAccounts({ env: agent => ({ ...process.env, ...this.activeRegistry.get(agent).env }) }),
      compaction: () => ({ atTokens: this.read<number>('compactAtTokens') ?? 300_000, auto: this.read<boolean>('autoCompact') ?? true }),
      hidden: () => this.read<HiddenMap>('hiddenOptions') ?? {},
      scope: () => sanitizeSetting('sessionScope', this.read('sessionScope')),
    });

    // The settings page's backend: reads / writes acpira.*, scans agent inventories; every bridge gets a subscription
    this.settings = new SettingsCenter({
      read: key => platform.readSetting(key),
      write: (key, value) => platform.writeSetting(key, value),
      writeAppearance: (axis, value) => platform.writeSetting(`appearance.${axis}`, value),
      hostLanguage: () => platform.hostLanguage(),
      registry: () => this.manager.registry,
      runtimeInfo: agent => this.manager.runtimeInfo(agent),
      home: () => platform.home(),
      cwd: () => platform.cwd(),
    });

    this.unsubscribe.push(
      platform.onSettingsChanged(affects => this.settingsChanged(affects)),
      // Coming back from an external terminal where a CLI was installed or removed: re-check the executables right away; coming back from
      // another window (VS Code, Cursor or IDEA, same ~/.acpira): pick up the sessions and accounts it created or deleted
      platform.onWindowFocus(() => {
        void this.manager.reprobe();
        void this.manager.refreshIndex();
        accounts.reload().catch(e => log(`account reload failed: ${msg(e)}`));
      }),
    );
  }

  static async create(platform: HostPlatform, opts: HostRuntimeOpts = {}): Promise<HostRuntime> {
    const log = (line: string) => platform.log(line);
    // ~/.acpira (ACPIRA_HOME) holds accounts.json, secrets.json, sessions/, scratch/; a platform with a legacy tree has it copied once
    const root = opts.home ?? acpiraHome();
    const vault = new FileVault(join(root, 'secrets.json'), log);
    if (platform.legacy) await migrateOnce({ from: platform.legacy.from, to: root, oldVault: platform.legacy.vault, newVault: vault, log });
    const accountStore = new AccountStore(join(root, 'accounts.json'), vault, log);
    await accountStore.load();
    const runtime = new HostRuntime(platform, root, vault, accountStore);
    await runtime.manager.init();
    setHostLocale(runtime.settings.locale());
    return runtime;
  }

  private read<T>(key: string): T | undefined {
    return this.platform.readSetting(key) as T | undefined;
  }

  appearance(): Appearance {
    return appearanceFromSettings((k: AxisKey) => this.platform.readSetting(`appearance.${k}`));
  }

  // One BridgeCore per view; the caller owns the transport and calls dispose() on the returned core when the view goes away
  attachView(opts: BridgeCoreOpts & { host: WebviewHost }): BridgeCore {
    const core = new BridgeCore({ manager: this.manager, settings: this.settings, platform: this.platform, appearance: () => this.appearance() }, opts);
    this.bridges.add(core);
    return core;
  }

  detachView(core: BridgeCore) {
    if (this.bridges.delete(core)) core.dispose();
  }

  private settingsChanged(affects: SettingsAffects) {
    if (affects('appearance')) for (const b of this.bridges) b.pushAppearance();
    if (affects('agents')) { this.activeRegistry = new AgentRegistry(this.read<Record<string, CustomAgentSetting>>('agents') ?? {}); this.manager.setRegistry(this.activeRegistry); }
    if (affects('hiddenOptions')) this.manager.emitHidden();
    // Any knob the settings page shows (language, defaultAgent, compaction, …): re-push the view and follow a language change host-side.
    // Checked per key, not as "anything but appearance / agents": one shell event may carry an appearance axis and a language change together
    if (SETTING_KEYS.some(k => affects(k))) {
      this.settings.emit();
      setHostLocale(this.settings.locale());
    }
  }

  async dispose() {
    for (const u of this.unsubscribe) u();
    this.unsubscribe = [];
    for (const b of this.bridges) b.dispose();
    this.bridges.clear();
    await this.manager.dispose().catch(e => this.platform.log(`dispose failed: ${msg(e)}`));
  }
}

export const createHostRuntime = HostRuntime.create;
