import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
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
  // Served through Antigravity on the gateway; its Chat answers carry reasoning_content, its Responses answers no
  // encrypted reasoning (2026-10-11)
  'gemini-3.8-flash': { id: 'gemini-3.8-flash', api: 'chat', catalog: 'gemini-3.8-flash', context: 1_048_576, output: 65_536 },
  // Devin's SWE model; not in the catalogue, so priced as Kimi K3. The gateway reports no cache reads for it
  'swe-2': { id: 'swe-2', api: 'responses', catalog: 'kimi-k3', context: 262_144, output: 32_000 },
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
}

export interface HarnessSetup {
  agent: AgentId;
  env: Record<string, string>;
}

const json = (path: string, value: unknown) => {
  mkdirSync(join(path, '..'), { recursive: true });
  writeFileSync(path, JSON.stringify(value, null, 2) + '\n');
};

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

  codex({ model, base, home }) {
    mkdirSync(join(home, '.codex'), { recursive: true });
    writeFileSync(join(home, '.codex/config.toml'), [
      `model = "${model.id}"`,
      'model_provider = "evalgw"',
      'model_reasoning_effort = "high"',
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
