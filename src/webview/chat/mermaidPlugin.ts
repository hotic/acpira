import type { DiagramPlugin, MermaidConfig, MermaidInstance } from '@streamdown/mermaid';

// @streamdown/mermaid's plugin, loaded when the first diagram renders. The package imports mermaid statically, which put
// about 5 MB of diagram code (mermaid, cytoscape, d3, marked …) into the page's startup bundle for the rare reply that
// draws one. Rendering is asynchronous either way: MermaidBlock waits 200 ms before its first try and shows its
// placeholder until the SVG arrives, so the load hides inside that wait. The package's own instance does the work once
// it is here (same defaults, same config merge)
let loading: Promise<MermaidInstance> | undefined;
const load = () => loading ??= import('@streamdown/mermaid').then(m => m.mermaid.getMermaid());

function createMermaidPlugin(): DiagramPlugin {
  // The config of the latest initialize, applied at the next render: it may come before the package has loaded
  let config: MermaidConfig | undefined;
  const instance: MermaidInstance = {
    initialize(next) {
      config = next;
    },
    async render(id, source) {
      const mermaid = await load();
      if (config) {
        mermaid.initialize(config);
        config = undefined;
      }
      return mermaid.render(id, source);
    },
  };
  return {
    name: 'mermaid',
    type: 'diagram',
    language: 'mermaid',
    getMermaid(next) {
      if (next) instance.initialize(next);
      return instance;
    },
  };
}

export const mermaidDiagram = createMermaidPlugin();
