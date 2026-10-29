// Interface pieces arriving from a card (docs/dev/plugins.md Activation): applying an interface plugin lifts the lit
// block of its thumbnail's map, and when the piece shows up in its slot (HostMsg pluginUi) the block flies onto it
// (effects/magic.ts flyInto). Stale after a while, so a piece that turns up later for another reason just appears
const ARRIVAL_MS = 15_000;
const pending = new Map<string, { from: DOMRect; until: number }>();

export function expectArrival(plugin: string, slot: string, from: DOMRect) {
  pending.set(`${plugin}|${slot}`, { from, until: performance.now() + ARRIVAL_MS });
}

// Taken once, by the slot the piece lands in
export function takeArrival(plugin: string, slot: string): DOMRect | undefined {
  const key = `${plugin}|${slot}`;
  const a = pending.get(key);
  pending.delete(key);
  return a && a.until > performance.now() ? a.from : undefined;
}
