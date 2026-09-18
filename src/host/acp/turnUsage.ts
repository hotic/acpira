import type * as acp from '@agentclientprotocol/sdk';
import type { TurnUsage } from '@shared/transcript';

type TokenField = 'input' | 'output' | 'cachedRead' | 'cachedWrite' | 'reasoning' | 'total' | 'modelCalls';

// Whole-token counts only: anything that is not a finite non-negative number is dropped, never rounded into a fake value
function tokens(v: unknown): number | undefined {
  return typeof v === 'number' && Number.isFinite(v) && v >= 0 ? Math.round(v) : undefined;
}

function label(v: unknown): string | undefined {
  return typeof v === 'string' && v ? v : undefined;
}

function record(v: unknown): Record<string, unknown> | undefined {
  return v !== null && typeof v === 'object' ? v as Record<string, unknown> : undefined;
}

// Per-prompt token accounting off the session/prompt response: the standard (unstable) usage field first, then vendor _meta —
// Grok 1.0.18's flat counts plus its usage sub-object (modelCalls, cacheCreationTokens), its modelId / requestId, and Devin's
// cognition.ai/userMessageId. costUsdTicks is deliberately not read (unit unverified); context occupancy is not here either —
// usage_update stamps it on the turn afterwards.
export function turnUsageOf(r: acp.PromptResponse): TurnUsage | undefined {
  const out: TurnUsage = {};
  const fill = (field: TokenField, v: unknown) => {
    const n = tokens(v);
    if (n !== undefined && out[field] === undefined) out[field] = n;
  };
  // `out.model ??= …` would still create the key with an undefined value, so strings go through the same only-when-present rule
  const text = (field: 'model' | 'requestId', v: unknown) => {
    const s = label(v);
    if (s !== undefined && out[field] === undefined) out[field] = s;
  };

  const u = r.usage;
  if (u) {
    fill('input', u.inputTokens);
    fill('output', u.outputTokens);
    fill('total', u.totalTokens);
    fill('reasoning', u.thoughtTokens);
    fill('cachedRead', u.cachedReadTokens);
    fill('cachedWrite', u.cachedWriteTokens);
  }

  const meta = record(r._meta);
  if (meta) {
    fill('input', meta.inputTokens);
    fill('output', meta.outputTokens);
    fill('total', meta.totalTokens);
    fill('cachedRead', meta.cachedReadTokens);
    fill('reasoning', meta.reasoningTokens);
    const inner = record(meta.usage);
    if (inner) {
      fill('cachedWrite', inner.cacheCreationTokens);
      fill('modelCalls', inner.modelCalls);
    }
    text('model', meta.modelId);
    text('requestId', meta.requestId);
    text('requestId', meta['cognition.ai/userMessageId']);
  }

  return Object.keys(out).length ? out : undefined;
}
