import { describe, expect, it } from 'vitest';
import { availableChromeCommands, columnPicks, columnsLayout } from '../src/host/multiOpen';

describe('open in editor columns', () => {
  it('offers 2 to 6 columns with 4 preselected', () => {
    const picks = columnPicks();
    expect(picks.map(p => p.columns)).toEqual([2, 3, 4, 5, 6]);
    expect(picks.filter(p => p.picked).map(p => p.columns)).toEqual([4]);
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
