import { cpSync, existsSync, mkdirSync, readFileSync, symlinkSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import type { AgentId } from '@shared/transcript';

// How each harness under comparison is pointed at the metering proxy. Every run gets a fresh HOME, so the configs written
// here are the only ones a harness sees: the user's own ~/.config/opencode, ~/.pi, ~/.codex, ~/.claude and ~/.agents stay
// out of reach, global instruction files included. Keys are a dummy; the proxy sets the real one (meter.ts)

export type Api = 'chat' | 'anthropic' | 'responses';

export interface ModelSpec {
  // The upstream's model id
  id: string;
  // Which upstream the proxy forwards to (meter.ts UPSTREAMS): the gateway unless set
  upstream?: 'gw' | 'ds';
  // The wire a model is reached on where the harness gets to choose (OpenCode, Pi); Codex always uses Responses, Claude
  // Code and the built-in agent's Anthropic format always use Messages
  api: Api;
  // Model catalogue id (rust/crates/acpira-shared/assets/model-catalog.json), for prices
  catalog: string;
  // USD per million tokens, for a model the catalogue does not list yet
  price?: { input: number; output: number; cacheRead?: number };
  context: number;
  output: number;
}

export const MODELS: Record<string, ModelSpec> = {
  'glm-5.3': { id: 'glm-5.3', api: 'chat', catalog: 'glm-5.3', context: 200_000, output: 32_000 },
  'qwen3.8-max': { id: 'qwen3.8-max', api: 'chat', catalog: 'qwen3.8-max', context: 200_000, output: 32_000 },
  'deepseek-v4-pro': { id: 'deepseek-v4-pro', api: 'chat', catalog: 'deepseek-v4-pro', context: 1_000_000, output: 32_000 },
  // The dashed id, which Claude Code recognises; the gateway answers both spellings from the same upstream, which ends the
  // stream at a tool call with empty input (docs/dev/builtin-agent.md, real providers)
  'claude-sonnet-5.5': { id: 'claude-sonnet-5-5', api: 'anthropic', catalog: 'claude-sonnet-5-5', context: 200_000, output: 32_000 },
  'gpt-6.1-sol': { id: 'gpt-6.1-sol', api: 'responses', catalog: 'gpt-6.1-sol', context: 400_000, output: 32_000 },
  // The review bench's reviewer and the Fusion pairs' other halves (all listed by the gateway's /v1/models on 2026-10-21)
  'gpt-6-astra': { id: 'gpt-6-astra', api: 'responses', catalog: 'gpt-6-astra', context: 1_050_000, output: 128_000 },
  'gpt-6-luna': { id: 'gpt-6-luna', api: 'responses', catalog: 'gpt-6-luna', context: 1_050_000, output: 128_000 },
  'claude-opus-5.5': { id: 'claude-opus-5-5', api: 'anthropic', catalog: 'claude-opus-5-5', context: 1_000_000, output: 128_000 },
  // Served through Antigravity on the gateway; its Chat answers carry reasoning_content, its Responses answers no
  // encrypted reasoning (2026-10-11)
  'gemini-3.8-flash': { id: 'gemini-3.8-flash', api: 'chat', catalog: 'gemini-3.8-flash', context: 1_048_576, output: 65_536 },
  // Pinned to the gateway's Antigravity channel (the bare name falls back to other backends when its one account is
  // out of quota). Not in the catalogue yet: priced at Google's launch rate. The channel caps every reply at 32768
  // output tokens whatever the request asks, and the high tier can spend all of it thinking (2026-10-15)
  'gemini-4-argon': {
    id: 'antigravity/gemini-4-argon', api: 'responses', catalog: 'gemini-4-argon', context: 1_048_576, output: 32_768,
    price: { input: 2, output: 10, cacheRead: 0.1 },
  },
  // Devin's SWE model; not in the catalogue, so priced as Kimi K3. The gateway reports no cache reads for it
  'swe-2': { id: 'swe-2', api: 'responses', catalog: 'kimi-k3', context: 262_144, output: 32_000 },
  // V4.1 Flash on the gateway (2026-10-18), for when there is no DeepSeek key; its cache numbers are unchecked
  'deepseek-v4.1-flash': { id: 'deepseek-v4.1-flash', api: 'chat', catalog: 'deepseek-flash', context: 1_000_000, output: 32_000 },
  // DeepSeek's own API (deepseek-flash is V4.1 Flash), whose usage reports real cache hits
  'deepseek-flash': { id: 'deepseek-flash', upstream: 'ds', api: 'chat', catalog: 'deepseek-flash', context: 1_000_000, output: 32_000 },
};

export const DUMMY_KEY = 'eval-dummy-key';

export interface HarnessCtx {
  model: ModelSpec;
  // http://127.0.0.1:PORT/r/<run>/gw: the gateway root behind the proxy, without /v1
  base: string;
  // The run's HOME
  home: string;
  // The sidecar's data root (where the built-in agent reads providers.json)
  acpiraHome: string;
  // A reasoning effort other than the harness's default, where its config sets one (Codex)
  effort?: string;
}

export interface HarnessSetup {
  agent: AgentId;
  env: Record<string, string>;
  // A harness that is its own CLI and no ACP agent: run once in the work tree, the prompt appended to `cmd`
  cli?: { cmd: string[]; env: Record<string, string> };
  // The agent talks to its own backend, so the metering proxy sees none of its calls
  unmetered?: boolean;
  // The login the agent needs is the CLI's own, imported into the run's sidecar home through the account menu's Import
  // before the session starts (Devin: its ACP mode ignores the CLI's credentials file)
  importLogin?: boolean;
}

const json = (path: string, value: unknown) => {
  mkdirSync(join(path, '..'), { recursive: true });
  writeFileSync(path, JSON.stringify(value, null, 2) + '\n');
};

// The macOS keychain as the run's HOME sees it: CLIs that keep their login there find it through HOME
function linkKeychain(home: string) {
  mkdirSync(join(home, 'Library'), { recursive: true });
  if (!existsSync(join(home, 'Library/Keychains'))) symlinkSync(join(homedir(), 'Library/Keychains'), join(home, 'Library/Keychains'));
}

const FORMATS: Record<Api, string> = { chat: 'openai-chat', responses: 'openai-responses', anthropic: 'anthropic' };

// The built-in agent on the model's own wire, or on `api` when given (the Chat-versus-Responses comparison)
function acpira({ model, base, acpiraHome }: HarnessCtx, efforts?: string[], api?: Api): HarnessSetup {
  json(join(acpiraHome, 'providers.json'), {
    version: 1,
    providers: [{
      id: 'evalgw', name: 'Eval gateway', preset: 'custom',
      format: FORMATS[api ?? model.api],
      baseUrl: `${base}/v1`,
      models: [{ id: model.id, enabled: true, ...(efforts ? { efforts } : {}) }],
    }],
  });
  json(join(acpiraHome, 'secrets.json'), { 'acpira.provider.evalgw': DUMMY_KEY });
  return { agent: 'acpira', env: {} };
}

export const HARNESSES: Record<string, (c: HarnessCtx) => HarnessSetup> = {
  acpira: c => acpira(c),
  // The built-in agent at a fixed effort: a model whose only level is this one starts there (the default is high)
  'acpira-high': c => acpira(c, ['high']),
  'acpira-medium': c => acpira(c, ['medium']),
  'acpira-low': c => acpira(c, ['low']),
  'acpira-chat': c => acpira(c, undefined, 'chat'),
  'acpira-responses': c => acpira(c, undefined, 'responses'),

  opencode({ model, base, home }) {
    const npm = model.api === 'anthropic' ? '@ai-sdk/anthropic' : model.api === 'responses' ? '@ai-sdk/openai' : '@ai-sdk/openai-compatible';
    json(join(home, '.config/opencode/opencode.json'), {
      $schema: 'https://opencode.ai/config.json',
      model: `evalgw/${model.id}`,
      small_model: `evalgw/${model.id}`,
      autoupdate: false,
      share: 'disabled',
      provider: {
        evalgw: {
          npm,
          name: 'Eval gateway',
          options: { baseURL: `${base}/v1`, apiKey: DUMMY_KEY },
          models: { [model.id]: { name: model.id, reasoning: true, tool_call: true, limit: { context: model.context, output: model.output } } },
        },
      },
    });
    return { agent: 'opencode', env: {} };
  },

  pi({ model, base, home }) {
    const api = model.api === 'anthropic' ? 'anthropic-messages' : model.api === 'responses' ? 'openai-responses' : 'openai-completions';
    json(join(home, '.pi/agent/models.json'), {
      providers: {
        evalgw: {
          baseUrl: model.api === 'anthropic' ? base : `${base}/v1`,
          api,
          apiKey: DUMMY_KEY,
          ...(model.api === 'chat' ? { compat: { supportsDeveloperRole: false } } : {}),
          models: [{ id: model.id, name: model.id, reasoning: true, input: ['text'], contextWindow: model.context, maxTokens: model.output }],
        },
      },
    });
    json(join(home, '.pi/agent/settings.json'), { defaultProvider: 'evalgw', defaultModel: model.id, defaultThinkingLevel: 'high' });
    return { agent: 'pi', env: {} };
  },

  kimi({ model, base, home }) {
    // Kimi Code takes any provider wire; its own Moonshot login is not used
    const type = model.api === 'anthropic' ? 'anthropic' : model.api === 'responses' ? 'openai_responses' : 'openai';
    mkdirSync(join(home, '.kimi-code'), { recursive: true });
    writeFileSync(join(home, '.kimi-code/config.toml'), [
      `default_model = "evalgw/${model.id}"`,
      '',
      '[providers.evalgw]',
      `type = "${type}"`,
      `base_url = "${model.api === 'anthropic' ? base : `${base}/v1`}"`,
      `api_key = "${DUMMY_KEY}"`,
      '',
      `[models."evalgw/${model.id}"]`,
      'provider = "evalgw"',
      `model = "${model.id}"`,
      `max_context_size = ${model.context}`,
      `max_output_size = ${model.output}`,
      'capabilities = ["thinking", "tool_use"]',
      '',
      '[thinking]',
      'enabled = true',
      'effort = "high"',
      '',
    ].join('\n'));
    return { agent: 'kimi', env: {} };
  },

  // Codex on the gateway. On this machine Codex's own login is this same gateway (a custom provider in ~/.codex), so
  // this metered harness also stands for "Codex on its own login"
  codex({ model, base, home, effort }) {
    mkdirSync(join(home, '.codex'), { recursive: true });
    writeFileSync(join(home, '.codex/config.toml'), [
      `model = "${model.id}"`,
      'model_provider = "evalgw"',
      `model_reasoning_effort = "${effort ?? 'high'}"`,
      '',
      '[model_providers.evalgw]',
      'name = "evalgw"',
      `base_url = "${base}/v1"`,
      'wire_api = "responses"',
      `experimental_bearer_token = "${DUMMY_KEY}"`,
      'supports_websockets = false',
      'requires_openai_auth = false',
      '',
    ].join('\n'));
    return { agent: 'codex', env: {} };
  },

  // Google's own harness on the same model: the Antigravity CLI in print mode, which is the bench's "Antigravity CLI"
  // row, with the CLI's Google sign-in. The run's HOME gets only that sign-in: the auth files under ~/.gemini and a link
  // to the macOS keychain (agy keeps its token there and finds the keychain through HOME), not ~/.gemini/config with
  // global rules, skills and MCP servers. BROWSER is a no-op, so a lost sign-in fails the run instead of opening a
  // browser. Model ids are Antigravity's own tiers, so this ignores `model` and runs Gemini 4 Argon at its highest tier
  agy({ home }) {
    const real = join(homedir(), '.gemini');
    for (const f of ['oauth_creds.json', 'google_accounts.json', 'installation_id', 'jetski-standalone-oauth-token', 'settings.json', 'antigravity-cli/installation_id', 'antigravity-cli/jetski_state.pbtxt']) {
      if (!existsSync(join(real, f))) continue;
      mkdirSync(dirname(join(home, '.gemini', f)), { recursive: true });
      cpSync(join(real, f), join(home, '.gemini', f));
    }
    linkKeychain(home);
    return {
      agent: 'antigravity', env: {}, unmetered: true,
      cli: { cmd: ['agy', '--model', 'gemini-4-argon-high', '--dangerously-skip-permissions', '--output-format', 'json', '-p'], env: { BROWSER: '/usr/bin/true' } },
    };
  },

  // Claude Code on its own claude.ai (Max) login, unmetered. Claude Code keeps the OAuth token in the macOS keychain
  // under a service name derived from its secure-storage directory, unsuffixed when no CLAUDE_CONFIG_DIR /
  // CLAUDE_SECURESTORAGE_CONFIG_DIR is set, so the run's HOME gets the default config dir, a link to the keychain and
  // only the onboarding and account entries of ~/.claude.json (no settings, CLAUDE.md, skills, MCP servers or
  // projects). A token refresh writes the same keychain item the user's own CLI reads. Checked 2026-10-21 with Claude
  // Code 2.1.299: `claude auth status` in such a HOME is logged in through claude.ai. The model comes from the
  // persona or the harness caller (`opus`, `fable`, …), not from `model`
  'claude-max'({ home }) {
    linkKeychain(home);
    const real = join(homedir(), '.claude.json');
    const own = existsSync(real) ? JSON.parse(readFileSync(real, 'utf8')) as Record<string, unknown> : {};
    json(join(home, '.claude.json'), {
      hasCompletedOnboarding: true,
      ...(own.lastOnboardingVersion ? { lastOnboardingVersion: own.lastOnboardingVersion } : {}),
      ...(own.oauthAccount ? { oauthAccount: own.oauthAccount } : {}),
    });
    return { agent: 'claude', env: { CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1', DISABLE_AUTOUPDATER: '1' }, unmetered: true };
  },

  // Devin on its own login (the user's Devin plan), unmetered: its ACP mode takes the API key only through
  // `authenticate`, so the login is imported into the run's sidecar home (`importLogin`) and the session binds to it.
  // The run's HOME keeps Devin's config and data apart from the user's (rules, MCP servers, sessions). Which model or
  // Fusion pair runs is the caller's pick on the session's model select (`run.ts --pick`)
  devin() {
    return { agent: 'devin', env: {}, unmetered: true, importLogin: true };
  },

  claude({ model, base, home }) {
    return {
      agent: 'claude',
      env: {
        CLAUDE_CONFIG_DIR: join(home, '.claude'),
        ANTHROPIC_BASE_URL: base,
        ANTHROPIC_AUTH_TOKEN: DUMMY_KEY,
        // The dashed catalogue id, which Claude Code recognises (the gateway takes both spellings)
        ANTHROPIC_MODEL: model.catalog,
        // Background calls (titles, summaries) go to the same model: the gateway has no Haiku route
        ANTHROPIC_DEFAULT_HAIKU_MODEL: model.catalog,
        ANTHROPIC_SMALL_FAST_MODEL: model.catalog,
        CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1',
        DISABLE_AUTOUPDATER: '1',
      },
    };
  },
};
