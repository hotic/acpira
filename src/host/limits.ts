// Session title taken from the first prompt; rename uses a wider cap so a hand-typed name isn't clipped as aggressively
export const TITLE_MAX = 40;
export const RENAME_MAX = 80;
// Tool output parked on a transcript block; beyond this only the head is kept
export const TOOL_OUTPUT_MAX = 20_000;
// Full-history editing is a fallback, not native compaction. Bound its wire
// payload before replacing a session; this is not a model token-window estimate.
export const EDIT_CONTEXT_MAX_BYTES = 256 * 1024;
// Plan-file preview loaded before showing approval; larger files keep the permission UI without a body
export const PLAN_PREVIEW_MAX_BYTES = 1_048_576;
