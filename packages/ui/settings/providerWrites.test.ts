import { describe, expect, it, vi } from 'vitest';
import { newModel, type ProviderAction } from '@acpira/shared/providers';
import { ProviderWrites } from './providerWrites';

const save = (id: string, enabled: boolean, key?: string): ProviderAction => ({ kind: 'save', key,
  provider: { id, name: id, preset: 'custom', format: 'openai-chat', baseUrl: 'https://example.test/v1', fullUrl: false, enabled: true,
    models: [{ ...newModel('model'), enabled }] },
});

function setup() {
  const send = vi.fn();
  const read = vi.fn();
  return { send, read, writes: new ProviderWrites(send, read) };
}

describe('provider writes', () => {
  it('keeps the latest draft across older replies and serializes different sources', () => {
    const { writes, send } = setup();
    const first = save('a', false), second = save('b', false), latest = save('a', true);
    writes.push(first, true);
    writes.push(second, true);
    writes.push(latest, true);
    expect(send.mock.calls).toEqual([[first]]);
    expect(writes.drafts.a?.models[0]?.enabled).toBe(true);
    expect(writes.receive()).toBe(first);
    expect(writes.drafts.a?.models[0]?.enabled).toBe(true);
    expect(send.mock.calls).toEqual([[first], [second]]);
    expect(writes.receive()).toBe(second);
    expect(writes.drafts.b).toBeUndefined();
    expect(writes.receive()).toBe(latest);
    expect(writes.drafts).toEqual({});
    expect(writes.pending).toBe(false);
    expect(send.mock.calls).toEqual([[first], [second], [latest]]);
  });

  it('coalesces consecutive toggles but never drops a credential save', () => {
    const { writes, send } = setup();
    const first = save('a', false), stale = save('a', true), latest = save('a', false);
    const credentials = save('a', true, 'test-key');
    writes.push(first, true);
    writes.push(stale, true);
    writes.push(latest, true);
    writes.push(credentials);
    writes.push(stale, true);
    while (writes.pending) writes.receive();
    expect(send.mock.calls).toEqual([[first], [latest], [credentials], [stale]]);
  });

  it('releases a failed draft on its reply without rolling back newer intent', () => {
    const { writes } = setup();
    writes.push(save('a', false), true);
    writes.push(save('a', true), true);
    writes.receive();
    expect(writes.drafts.a?.models[0]?.enabled).toBe(true);
    writes.receive();
    expect(writes.drafts).toEqual({});
    // The caller renders the reply's authoritative view, unchanged when the save failed.
  });

  it('holds a click behind a pending read and suppresses refreshes during saves', () => {
    const { writes, send, read } = setup();
    writes.refresh();
    writes.refresh();
    const action = save('a', false);
    writes.push(action, true);
    expect(read).toHaveBeenCalledTimes(1);
    expect(send).not.toHaveBeenCalled();
    expect(writes.drafts.a?.models[0]?.enabled).toBe(false);
    expect(writes.receive()).toBeUndefined();
    expect(send).toHaveBeenCalledWith(action);
    writes.refresh();
    expect(read).toHaveBeenCalledTimes(1);
    expect(writes.receive()).toBe(action);
    writes.refresh();
    expect(read).toHaveBeenCalledTimes(2);
  });
});
