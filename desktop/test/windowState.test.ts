import { describe, expect, it } from 'vitest';
import { parseWindowsState, placeable } from '../src/windowState';

describe('window state', () => {
  it('keeps what parses and drops the rest', () => {
    expect(parseWindowsState({
      windows: [
        { bounds: { x: 10, y: 20, width: 800, height: 600 }, maximized: true, session: 's1' },
        { bounds: { x: 0, y: 0, width: 0, height: 600 }, session: '' },
        'junk',
      ],
      last: { bounds: { x: 'a' } },
    })).toEqual({ windows: [{ bounds: { x: 10, y: 20, width: 800, height: 600 }, maximized: true, session: 's1' }, {}], last: {} });
    expect(parseWindowsState(undefined)).toEqual({ windows: [] });
  });

  const main = { x: 0, y: 0, width: 1440, height: 875 };
  const side = { x: 1440, y: -200, width: 1920, height: 1055 };

  it('keeps bounds whose title bar lies on a display', () => {
    const b = { x: 1500, y: 0, width: 1000, height: 700 };
    expect(placeable(b, [main, side])).toEqual(b);
  });

  it('keeps only the size, capped to the largest display, when the old place is off screen', () => {
    expect(placeable({ x: 1500, y: 0, width: 2400, height: 1400 }, [main])).toEqual({ width: 1440, height: 875 });
    expect(placeable({ x: -3000, y: 100, width: 900, height: 700 }, [main, side])).toEqual({ width: 900, height: 700 });
    expect(placeable({ x: 0, y: 0, width: 900, height: 700 }, [])).toEqual({ width: 900, height: 700 });
  });
});
