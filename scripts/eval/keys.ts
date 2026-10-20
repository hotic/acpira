import { readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

// Keys the evaluation scripts spend, read where they already live; never printed or written to a run's files

// The gateway: the `asgard` provider of Codex's config
export const GATEWAY = 'https://ai.sacredcraft.cn';

export function gatewayKey(): string {
  if (process.env.EVAL_GW_KEY) return process.env.EVAL_GW_KEY;
  const toml = readFileSync(join(homedir(), '.codex/config.toml'), 'utf8');
  const section = toml.split(/^\[/m).find(s => s.startsWith('model_providers.asgard]'));
  const m = section && /experimental_bearer_token\s*=\s*"([^"]+)"/.exec(section);
  if (!m) throw new Error('no gateway key (EVAL_GW_KEY or ~/.codex/config.toml model_providers.asgard)');
  return m[1]!;
}

// DeepSeek's own key, only when a model goes there: EVAL_DS_KEY, or the file EVAL_DS_KEY_FILE names
export function deepseekKey(): string | undefined {
  if (process.env.EVAL_DS_KEY) return process.env.EVAL_DS_KEY;
  return process.env.EVAL_DS_KEY_FILE ? readFileSync(process.env.EVAL_DS_KEY_FILE, 'utf8').trim() : undefined;
}
