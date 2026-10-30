// Interface pieces arriving from a card (docs/dev/plugins.md Activation): applying an interface plugin lifts the lit
// block of its thumbnail's map, and when the piece shows up in its slot (HostMsg pluginUi) the block flies onto it
// (effects/magic.ts flyInto). Stale after a while, so a piece that turns up later for another reason just appears
const ARRIVAL_MS = 15_000;
const pending = new Map<string, { from: DOMRect; until: number; session?: string }>();

// `session`: the card's; a slot inside a session (the composer's toolbar) takes only its own session's arrival, so one
// applied there and then left does not fly onto the next session's composer from where the card was
export function expectArrival(plugin: string, slot: string, from: DOMRect, session?: string) {
  pending.set(`${plugin}|${slot}`, { from, until: performance.now() + ARRIVAL_MS, session });
}

// Taken once, by the slot the piece lands in
export function takeArrival(plugin: string, slot: string, session?: string): DOMRect | undefined {
  const key = `${plugin}|${slot}`;
  const a = pending.get(key);
  if (a && session !== undefined && a.session !== undefined && a.session !== session) return undefined;
  pending.delete(key);
  return a && a.until > performance.now() ? a.from : undefined;
}
