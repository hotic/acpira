import { describe, expect, it } from 'vitest';
import { questionOptionIndex } from '../chat/questionKeys';

describe('question option shortcuts', () => {
  it.each(['Tab', 'Shift', 'Enter', 'Escape', 'ArrowLeft', 'ArrowRight', 'Home', 'End', 'F1', 'Dead', 'Process', '', ' ', '0', '中'])
    ('does not turn %j into an answer', key => {
      expect(questionOptionIndex(key)).toBeUndefined();
    });

  it.each([['a', 0], ['A', 0], ['z', 25], ['1', 0], ['9', 8]] as const)
    ('maps the displayed shortcut %s to option %i', (key, index) => {
      expect(questionOptionIndex(key)).toBe(index);
    });
});
