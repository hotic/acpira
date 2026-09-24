import { readFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';

// The Devin CLI's own login ($XDG_DATA_HOME/devin/credentials.toml), which its ACP mode ignores: a probe hands it over through
// `authenticate` `_meta.api_key` the way the account layer does. Only the flat `key = "value"` shape of that file is read
export interface DevinLogin { secret: string; apiServerUrl?: string }

export async function readDevinLogin(): Promise<DevinLogin | undefined> {
  const file = join(process.env.XDG_DATA_HOME || join(homedir(), '.local', 'share'), 'devin', 'credentials.toml');
  const text = await readFile(file, 'utf8').catch(() => undefined);
  if (!text) return undefined;
  const keys = Object.fromEntries([...text.matchAll(/^\s*(\w+)\s*=\s*"([^"]*)"\s*$/gm)].map(m => [m[1], m[2]]));
  return keys.windsurf_api_key ? { secret: keys.windsurf_api_key, apiServerUrl: keys.api_server_url || undefined } : undefined;
}

export function devinAuthenticate(methodId: string | undefined, login: DevinLogin) {
  const meta: Record<string, string> = { api_key: login.secret };
  if (login.apiServerUrl) meta.api_server_url = login.apiServerUrl;
  return { methodId: methodId ?? 'devin-browser', _meta: meta };
}
