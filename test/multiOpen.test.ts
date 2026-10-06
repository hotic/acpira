import { describe, expect, it } from 'vitest';
import { availableChromeCommands, columnPicks, columnsLayout, defaultColumnPick } from '../src/host/multiOpen';

describe('open in editor columns', () => {
  it('offers 2 to 6 columns and starts on 4', () => {
    const picks = columnPicks();
    expect(picks.map(p => p.columns)).toEqual([2, 3, 4, 5, 6]);
    // `picked` is multi-select only in VS Code; the default is the picker's active item instead
    expect(picks.some(p => 'picked' in p)).toBe(false);
    expect(defaultColumnPick(picks)).toBe(picks[2]);
  });

  it('lays out one row of equal-width groups', () => {
    const layout = columnsLayout(3);
    expect(layout.orientation).toBe(0);
    expect(layout.groups).toHaveLength(3);
    for (const g of layout.groups) expect(g.size).toBeCloseTo(1 / 3);
  });

  it('skips chrome commands the host does not register, keeping the order', () => {
    expect(availableChromeCommands(['workbench.action.closePanel', 'workbench.action.closeSidebar'])).toEqual(['workbench.action.closeSidebar', 'workbench.action.closePanel']);
    expect(availableChromeCommands([])).toEqual([]);
  });
});
