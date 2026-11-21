import { readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

// Keys the evaluation scripts spend, read where they already live; never printed or written to a run's files

// The gateway: the `asgard` provider of Codex's config. Its address and key are read from there or the environment,
// neither is written in the repository
function codexGateway(field: string): string | undefined {
  let toml: string;
  try { toml = readFileSync(join(homedir(), '.codex/config.toml'), 'utf8'); } catch { return undefined; }
  const section = toml.split(/^\[/m).find(s => s.startsWith('model_providers.asgard]'));
  return (section && new RegExp(`${field}\\s*=\\s*"([^"]+)"`).exec(section)?.[1]) || undefined;
}

// The gateway's origin, without the /v1 its callers add: EVAL_GW_URL, or the provider's base_url
export function gatewayUrl(): string {
  const url = process.env.EVAL_GW_URL ?? codexGateway('base_url');
  if (!url) throw new Error('no gateway address (EVAL_GW_URL or ~/.codex/config.toml model_providers.asgard base_url)');
  return url.replace(/\/v1\/?$/, '');
}

export function gatewayKey(): string {
  const key = process.env.EVAL_GW_KEY ?? codexGateway('experimental_bearer_token');
  if (!key) throw new Error('no gateway key (EVAL_GW_KEY or ~/.codex/config.toml model_providers.asgard)');
  return key;
}

// DeepSeek's own key, only when a model goes there: EVAL_DS_KEY, or the file EVAL_DS_KEY_FILE names
export function deepseekKey(): string | undefined {
  if (process.env.EVAL_DS_KEY) return process.env.EVAL_DS_KEY;
  return process.env.EVAL_DS_KEY_FILE ? readFileSync(process.env.EVAL_DS_KEY_FILE, 'utf8').trim() : undefined;
}
