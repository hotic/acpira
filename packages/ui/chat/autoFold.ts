import { createContext, createElement, useContext, useSyncExternalStore, type ReactNode } from 'react';

// Lets the turn drive the folds of its process items (open while live, closed once done, a manual toggle wins)
// without owning each row's state. The owner writes the whole map during its render, before the rows below it
// render, and notifies after commit, so a memoized row whose block did not change still hears about its own fold
// and nothing else re-renders: a context value would re-render every row of the live turn on each stream chunk.
// Rows outside a provider, or items the owner leaves out, keep their own local state.
export interface AutoFold {
  open: boolean;
  onToggle: (open: boolean) => void;
}

export class AutoFoldStore {
  private folds = new Map<string, boolean>();
  private listeners = new Set<() => void>();
  constructor(public toggle: (id: string, open: boolean) => void) {}
  write(next: Map<string, boolean>) { this.folds = next; }
  notify() { for (const listener of this.listeners) listener(); }
  get = (id: string | undefined) => (id === undefined ? undefined : this.folds.get(id));
  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };
}

export const AutoFoldContext = createContext<AutoFoldStore | undefined>(undefined);
// The process item a row belongs to; nested rows of a group keep their own state
const ItemContext = createContext<string | undefined>(undefined);

export function AutoFoldItem({ id, children }: { id: string; children: ReactNode }) {
  return createElement(ItemContext.Provider, { value: id }, children);
}

const idle = () => () => {};

export function useAutoFold(): AutoFold | undefined {
  const store = useContext(AutoFoldContext);
  const id = useContext(ItemContext);
  const open = useSyncExternalStore(store?.subscribe ?? idle, () => store?.get(id));
  if (!store || id === undefined || open === undefined) return;
  return { open, onToggle: next => store.toggle(id, next) };
}
