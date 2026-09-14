import { execFile } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';
import * as acp from '@agentclientprotocol/sdk';
import type { AccountQuota, QuotaWindow } from '@shared/transcript';
import type { AgentProcess } from '../acp/AgentProcess';
import type { AccountCredential, AccountDraft, AccountProvider, LoginFlow } from './types';
import { t } from '../i18n';
import { VERSION } from '../version';

// Devin CLI login = a PKCE exchange for a long-lived API key, stored in $XDG_DATA_HOME/devin/credentials.toml (four keys).
// ACP mode does not read that file (so usage isn't billed to another account); the host must hand the key over via _meta.api_key in authenticate —
// that's what the Windsurf client inside Devin.app does (method windsurf-api-key + api_key / api_server_url).
// The browser login method (devin-browser) authenticates only the current process; the key is neither returned nor persisted, so a durable account can only come from the toml:
// import a local login, or run `devin auth login` once inside an isolated XDG directory and collect the toml.
// Identity (email / plan) comes from `devin auth status` reading the toml temporarily written into the isolated directory.
// Quota is not on the ACP wire (`/usage` is a TUI-only command): the CLI reads it from the Windsurf seat-management service, a Connect RPC that
// also answers JSON. `planStatus` carries daily / weekly remaining percent + reset times; `planInfo.hideDailyQuota` / `hideWeeklyQuota` say which
// windows the plan actually has (Max: weekly only; Pro: both). The three client fields in `metadata` are required or the server answers 400

const TOML_KEYS = ['api_server_url', 'devin_webapp_host', 'devin_api_url'] as const;
const SECRET_KEY = 'windsurf_api_key';
const DEFAULT_API_SERVER = 'https://server.codeium.com';
const USER_STATUS_PATH = '/exa.seat_management_pb.SeatManagementService/GetUserStatus';
const QUOTA_TIMEOUT = 10_000;

export class DevinAccountProvider implements AccountProvider {
  readonly agent = 'devin';

  // scratchDir: root of the temp directories used for isolated login / identity lookup; binary: locate the devin executable
  constructor(private scratchDir: string, private binary: () => Promise<string | null>) {}

  async importLocal(): Promise<AccountDraft | undefined> {
    const cred = await readCredentials(join(dataHome(), 'devin', 'credentials.toml'));
    if (!cred) return undefined;
    return { ...cred, ...(await this.identify(cred)) };
  }

  async login(): Promise<LoginFlow> {
    const bin = await this.binary();
    if (!bin) throw new Error(t('host.notFound', { command: 'devin', agent: 'Devin' }));
    const dir = join(this.scratchDir, `login-${randomUUID()}`);
    await mkdir(dir, { recursive: true, mode: 0o700 });
    const file = join(dir, 'devin', 'credentials.toml');
    return {
      command: bin, args: ['auth', 'login'],
      // ACP_BACKEND makes the CLI ignore local credentials; it must be removed from the login environment
      env: { XDG_DATA_HOME: dir, XDG_CONFIG_HOME: dir, ACP_BACKEND: null },
      collect: async signal => {
        try {
          const cred = await waitFor(() => readCredentials(file), signal);
          return cred && { ...cred, ...(await this.identify(cred)) };
        } finally { await rm(dir, { recursive: true, force: true }); }
      },
    };
  }

  // Write the credential into an isolated directory, run `devin auth status`, and parse email / plan / name; on failure, fall back to the key's last 4 chars as the label
  async identify(cred: AccountCredential): Promise<{ label: string; detail?: string }> {
    const fallback = { label: `Devin …${cred.secret.slice(-4)}` };
    const bin = await this.binary();
    if (!bin) return fallback;
    const dir = join(this.scratchDir, `whoami-${randomUUID()}`);
    try {
      await mkdir(join(dir, 'devin'), { recursive: true, mode: 0o700 });
      await writeFile(join(dir, 'devin', 'credentials.toml'), tomlOf(cred), { mode: 0o600 });
      const out = await run(bin, ['auth', 'status'], { XDG_DATA_HOME: dir, XDG_CONFIG_HOME: dir }, 20_000);
      return parseStatus(out) ?? fallback;
    } catch { return fallback; }
    finally { await rm(dir, { recursive: true, force: true }); }
  }

  async authenticate(proc: AgentProcess, cred: AccountCredential): Promise<void> {
    const methodId = proc.init.authMethods?.[0]?.id ?? 'devin-browser';
    const meta: Record<string, string> = { api_key: cred.secret };
    if (cred.meta?.api_server_url) meta.api_server_url = cred.meta.api_server_url;
    const req: acp.AuthenticateRequest = { methodId, _meta: meta };
    await proc.agent.request(acp.methods.agent.authenticate, req);
  }

  async quota(cred: AccountCredential): Promise<AccountQuota | undefined> {
    const base = (cred.meta?.api_server_url || DEFAULT_API_SERVER).replace(/\/$/, '');
    const res = await fetch(base + USER_STATUS_PATH, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'connect-protocol-version': '1' },
      body: JSON.stringify({ metadata: { apiKey: cred.secret, ideName: 'acpira', ideVersion: VERSION, extensionVersion: VERSION } }),
      signal: AbortSignal.timeout(QUOTA_TIMEOUT),
    });
    if (!res.ok) throw new Error(`GetUserStatus ${res.status}`);
    return parseUserStatus(await res.json());
  }
}

// GetUserStatus (JSON encoding) → the windows the plan exposes. Percentages arrive as whole numbers, reset times as unix seconds in strings (int64).
// proto3 JSON omits zero-valued fields, so on a quota-billed plan a missing percent is an exhausted window (0%), not a missing one; only
// `hideDailyQuota` / `hideWeeklyQuota` (or a plan not billed by quota) remove a window
export function parseUserStatus(json: unknown): AccountQuota | undefined {
  const status = (json as { userStatus?: { planStatus?: Record<string, unknown> } } | undefined)?.userStatus?.planStatus;
  if (!status) return undefined;
  const info = (status.planInfo ?? {}) as Record<string, unknown>;
  const quotaBilled = info.billingStrategy === 'BILLING_STRATEGY_QUOTA';
  const windows: QuotaWindow[] = [];
  const add = (id: string, hidden: unknown, pct: unknown, reset: unknown) => {
    if (hidden === true || (typeof pct !== 'number' && !quotaBilled)) return;
    const unix = Number(reset);
    windows.push({ id, remaining: Math.min(1, Math.max(0, (typeof pct === 'number' ? pct : 0) / 100)), resetsAt: Number.isFinite(unix) && unix > 0 ? new Date(unix * 1000).toISOString() : undefined });
  };
  add('daily', info.hideDailyQuota, status.dailyQuotaRemainingPercent, status.dailyQuotaResetAtUnix);
  add('weekly', info.hideWeeklyQuota, status.weeklyQuotaRemainingPercent, status.weeklyQuotaResetAtUnix);
  // GetUserStatus reports USD millionths as a signed int64 string. Keep negative balances
  // and explicit zero; an omitted balance provides no evidence of available credit.
  const rawBalance = status.overageBalanceMicros;
  const micros = typeof rawBalance === 'number' ? rawBalance
    : typeof rawBalance === 'string' && /^-?\d+$/.test(rawBalance) ? Number(rawBalance) : NaN;
  const onDemandBalanceUsd = Number.isSafeInteger(micros) ? micros / 1_000_000 : undefined;
  return windows.length || onDemandBalanceUsd !== undefined
    ? { windows, ...(onDemandBalanceUsd !== undefined && { onDemandBalanceUsd }), fetchedAt: new Date().toISOString() }
    : undefined;
}

export function dataHome(): string {
  return process.env.XDG_DATA_HOME || join(homedir(), '.local', 'share');
}

// Accepts only flat toml with one `key = "value"` per line — the shape of Devin's credentials file
export async function readCredentials(file: string): Promise<AccountCredential | undefined> {
  let text: string;
  try { text = await readFile(file, 'utf8'); } catch { return undefined; }
  const kv = new Map<string, string>();
  for (const line of text.split('\n')) {
    const m = /^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*"((?:[^"\\]|\\.)*)"\s*$/.exec(line);
    if (m) kv.set(m[1]!, m[2]!.replace(/\\(.)/g, '$1'));
  }
  const secret = kv.get(SECRET_KEY);
  if (!secret) return undefined;
  const meta: Record<string, string> = {};
  for (const k of TOML_KEYS) { const v = kv.get(k); if (v) meta[k] = v; }
  return { secret, meta };
}

export function tomlOf(cred: AccountCredential): string {
  const q = (v: string) => `"${v.replace(/[\\"]/g, '\\$&')}"`;
  const lines = [`${SECRET_KEY} = ${q(cred.secret)}`];
  for (const k of TOML_KEYS) if (cred.meta?.[k]) lines.push(`${k} = ${q(cred.meta[k]!)}`);
  return lines.join('\n') + '\n';
}

// `devin auth status` output is indented key-value lines like "  Email:   x@y". The name only falls back as the label
// when there is no email — the detail line is the plan tier alone
export function parseStatus(out: string): { label: string; detail?: string } | undefined {
  const field = (name: string) => new RegExp(`^\\s*${name}:\\s+(.+?)\\s*$`, 'm').exec(out)?.[1];
  const email = field('Email'), name = field('Name'), tier = field('Tier') ?? field('Plan');
  if (!email && !name) return undefined;
  return { label: email ?? name!, detail: tier };
}

function run(bin: string, args: string[], env: Record<string, string>, timeout: number): Promise<string> {
  const merged = { ...process.env, ...env };
  delete merged.ACP_BACKEND;
  return new Promise((resolve, reject) => {
    execFile(bin, args, { env: merged, timeout, maxBuffer: 1 << 20 }, (err, stdout) => (err ? reject(err) : resolve(stdout)));
  });
}

// Check once a second whether the file exists; give up as soon as the signal fires
async function waitFor<T>(read: () => Promise<T | undefined>, signal: AbortSignal): Promise<T | undefined> {
  while (!signal.aborted) {
    const v = await read();
    if (v) return v;
    await new Promise(r => setTimeout(r, 1000));
  }
  return undefined;
}
