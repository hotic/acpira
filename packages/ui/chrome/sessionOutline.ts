// What a plugin frame hears of the conversation on screen (acpira.onSession in /plugin-frame.html, docs/dev/plugins.md
// Interface): its shape per exchange, never its text, file names or commands, so a minimap or a progress strip can be
// drawn without the plugin reading the transcript. An exchange is a typed prompt and everything up to the next one
export interface ExchangeOutline {
  // The transcript index of the exchange's first turn: what acpira.scrollTo takes
  turn: number;
  edits: { files: number; add: number; del: number };
  runs: number;
  reads: number;
  tools: number;
  failed: number;
}

export interface SessionOutline {
  session: string;
  running: boolean;
  exchanges: ExchangeOutline[];
  // The exchanges at least partly in view, as transcript indices of their first turns
  seen?: [number, number];
}
