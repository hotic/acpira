import { describe, expect, it, vi } from 'vitest';
import { createRef } from 'react';
import { mergeRefs } from '../packages/ui/ui/mergeRefs';

describe('mergeRefs', () => {
  it('attaches one node to object and callback refs and detaches through the cleanup', () => {
    const object = createRef<string>();
    const callback = vi.fn();
    const detach = vi.fn();
    const withCleanup = vi.fn(() => detach);
    const cleanup = mergeRefs<string>(object, callback, undefined, withCleanup)('node');

    expect(object.current).toBe('node');
    expect(callback).toHaveBeenCalledWith('node');
    expect(withCleanup).toHaveBeenCalledWith('node');

    expect(typeof cleanup).toBe('function');
    (cleanup as () => void)();
    expect(object.current).toBeNull();
    expect(callback).toHaveBeenLastCalledWith(null);
    expect(detach).toHaveBeenCalledOnce();
  });
});
