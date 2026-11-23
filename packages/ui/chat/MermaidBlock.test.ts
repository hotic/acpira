import { describe, expect, it } from 'vitest';
import { parseSvgSize, prepareSvgForExport, scopeSvgIds } from './MermaidBlock';

describe('MermaidBlock helpers', () => {
  it('reads intrinsic dimensions from viewBox before width/height attributes', () => {
    const svg = '<svg id="acp-mmd-1" width="100%" viewBox="0 0 2145.5 412" style="max-width: 2145.5px;"></svg>';
    expect(parseSvgSize(svg)).toEqual({ width: 2145.5, height: 412 });
  });

  it('falls back to numeric width/height attributes when viewBox is absent', () => {
    expect(parseSvgSize('<svg width="640px" height="320"></svg>')).toEqual({ width: 640, height: 320 });
    expect(parseSvgSize('<svg width="100%"></svg>')).toBeUndefined();
  });

  it('scopes all SVG ids, markers, hrefs and style selectors without prefix collisions', () => {
    const svg = [
      '<svg id="acp-mmd-1" viewBox="0 0 100 100">',
      '<style>#acp-mmd-1{font-family:sans-serif;}#acp-mmd-1 .node{fill:#111;}</style>',
      '<defs><marker id="acp-mmd-1_flowchart-pointEnd"></marker></defs>',
      '<path marker-end="url(#acp-mmd-1_flowchart-pointEnd)" />',
      '<use href="#acp-mmd-1_flowchart-pointEnd" />',
      '</svg>',
    ].join('');
    const scoped = scopeSvgIds(svg, 'lb');
    expect(scoped).toContain('id="acp-mmd-1-lb"');
    expect(scoped).toContain('#acp-mmd-1-lb{font-family:sans-serif;}');
    expect(scoped).toContain('#acp-mmd-1-lb .node{fill:#111;}');
    expect(scoped).toContain('id="acp-mmd-1_flowchart-pointEnd-lb"');
    expect(scoped).toContain('marker-end="url(#acp-mmd-1_flowchart-pointEnd-lb)"');
    expect(scoped).toContain('href="#acp-mmd-1_flowchart-pointEnd-lb"');
  });

  it('stamps explicit pixel width, height and xmlns for PNG export', () => {
    const svg = '<svg id="acp-mmd-1" width="100%" viewBox="0 0 1200.4 300.2"><rect /></svg>';
    const out = prepareSvgForExport(svg);
    expect(out).toContain('xmlns="http://www.w3.org/2000/svg"');
    expect(out).toContain('width="1201"');
    expect(out).toContain('height="301"');
    expect(out).not.toContain('width="100%"');
  });
});
