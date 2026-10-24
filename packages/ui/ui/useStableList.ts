import { useRef } from 'react';

// Returns the previous array while its members are unchanged, so a list derived from
// ever-changing input (the transcript on each stream push) can feed memo deps and contexts
export function useStableList<T>(list: T[]): T[] {
  const kept = useRef(list);
  const previous = kept.current;
  if (previous !== list && (previous.length !== list.length || list.some((item, index) => item !== previous[index]))) kept.current = list;
  return kept.current;
}
