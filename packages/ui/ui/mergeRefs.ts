import { useMemo, type Ref, type RefCallback } from 'react';

// Wrappers that keep an internal ref must still honour the caller's `ref`; spreading props would otherwise replace it.
export function mergeRefs<T>(...refs: (Ref<T> | undefined)[]): RefCallback<T> {
  return node => {
    const cleanups = refs.map(ref => {
      if (typeof ref === 'function') return ref(node);
      if (ref) ref.current = node;
      return undefined;
    });
    return () => cleanups.forEach((cleanup, i) => {
      const ref = refs[i];
      if (typeof cleanup === 'function') cleanup();
      else if (typeof ref === 'function') ref(null);
      else if (ref) ref.current = null;
    });
  };
}

// Stable across renders while the inputs are, so React does not detach and reattach every commit.
export function useMergedRefs<T>(...refs: (Ref<T> | undefined)[]): RefCallback<T> {
  return useMemo(() => mergeRefs(...refs), refs);
}
