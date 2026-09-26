import { describe, expect, it } from 'vitest';
import { railStep } from '../src/webview/ui/railStep';

describe('railStep', () => {
  it('eases toward the target without overshooting', () => {
    const next = railStep(0, 100, 16, 55);
    expect(next).toBeGreaterThan(0);
    expect(next).toBeLessThan(100);
  });

  it('never moves away from the target when the elapsed time is negative', () => {
    // A frame timestamp earlier than the scheduling effect used to drive the rate negative and the length below zero
    expect(railStep(7, 100, -40, 55)).toBe(7);
    let length = 7;
    for (let i = 0; i < 20; i++) length = railStep(length, 100, -30, 55);
    expect(length).toBe(7);
  });

  it('snaps once within a quarter pixel', () => {
    expect(railStep(99.9, 100, 1, 55)).toBe(100);
  });
});
