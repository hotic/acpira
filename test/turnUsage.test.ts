import { describe, expect, it } from 'vitest';
import type * as acp from '@agentclientprotocol/sdk';
import { turnUsageOf } from '../src/host/acp/turnUsage';

// Prompt responses as each CLI actually sends them (verified on the wire); loose casts on purpose — _meta is untyped by definition
function response(v: unknown): acp.PromptResponse {
  return v as acp.PromptResponse;
}

describe('turnUsageOf', () => {
  it('reads the standard unstable usage field (Devin)', () => {
    expect(turnUsageOf(response({
      stopReason: 'end_turn',
      usage: { totalTokens: 120, inputTokens: 100, outputTokens: 20, cachedReadTokens: 64 },
      _meta: { 'cognition.ai/userMessageId': 'req-devin-1' },
    }))).toEqual({ input: 100, output: 20, total: 120, cachedRead: 64, requestId: 'req-devin-1' });
  });

  it('reads Grok\'s _meta: flat token counts, usage sub-object, modelId and requestId', () => {
    expect(turnUsageOf(response({
      stopReason: 'end_turn',
      _meta: {
        sessionId: 's1', requestId: 'req-grok-1', promptId: 'p1', modelId: 'grok-4.6',
        totalTokens: 38167, inputTokens: 38140, outputTokens: 20, cachedReadTokens: 37888, reasoningTokens: 19,
        usage: {
          inputTokens: 38140, outputTokens: 20, totalTokens: 38167, cachedReadTokens: 37888,
          cacheCreationTokens: 0, reasoningTokens: 19, modelCalls: 1,
          apiDurationMs: 1200, costUsdTicks: 42, modelUsage: {}, numTurns: 2,
        },
      },
    }))).toEqual({
      input: 38140, output: 20, total: 38167, cachedRead: 37888, reasoning: 19,
      cachedWrite: 0, modelCalls: 1, model: 'grok-4.6', requestId: 'req-grok-1',
    });
  });

  it('prefers the standard fields and does not let _meta overwrite them', () => {
    expect(turnUsageOf(response({
      stopReason: 'end_turn',
      usage: { totalTokens: 10, inputTokens: 8, outputTokens: 2 },
      _meta: { inputTokens: 999, modelId: 'm', requestId: 'r' },
    }))).toEqual({ input: 8, output: 2, total: 10, model: 'm', requestId: 'r' });
  });

  it('returns undefined for a response with no usage at all (Kimi)', () => {
    expect(turnUsageOf(response({ stopReason: 'end_turn' }))).toBeUndefined();
    expect(turnUsageOf(response({ stopReason: 'end_turn', _meta: { other: 'stuff' } }))).toBeUndefined();
  });

  it('drops garbage: negatives, strings, NaN, non-object metadata', () => {
    expect(turnUsageOf(response({
      stopReason: 'end_turn',
      usage: { totalTokens: NaN, inputTokens: -1, outputTokens: '12' },
      _meta: { inputTokens: 'x', usage: 'nope', modelId: 5, requestId: {}, modelCalls: '2' },
    }))).toBeUndefined();
  });

  it('keeps the usable fields when metadata is partially malformed', () => {
    expect(turnUsageOf(response({
      stopReason: 'end_turn',
      usage: { totalTokens: 30, inputTokens: -4, outputTokens: 10 },
      _meta: { usage: { modelCalls: 'many', cacheCreationTokens: 7 }, modelId: '' },
    }))).toEqual({ output: 10, total: 30, cachedWrite: 7 });
  });
});
