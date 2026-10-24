// Human-readable text of anything a catch clause receives: Error instances give their message, everything else is stringified
export function msg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
