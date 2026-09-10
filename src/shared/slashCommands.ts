import type { SessionControls, SlashCommand, Turn, TurnSettings } from './transcript';

// A command candidate must be a leading token, excluding multi-segment paths. Discovery
// is not a whitelist: unknown names still travel to the CLI unchanged.
export function commandName(text: string): string | undefined {
  return /^\/([\p{L}\p{N}][\p{L}\p{N}_.:-]*)(?=\s|$)/u.exec(text)?.[1];
}

export function namedCommand(commands: readonly SlashCommand[], text: string): SlashCommand | undefined {
  const name = commandName(text);
  return name ? commands.find(c => c.name === name) : undefined;
}

// Older records kept the native stop reason but no command receipt. Recover the
// empty-request explanation without inferring historical settings or success.
export function restoreCommandReceipts(turns: Turn[]): Turn[] {
  return turns.map((turn, i) => {
    const user = turns[i - 1];
    if (turn.role !== 'agent' || turn.command || turn.stop !== 'end_turn' || turn.blocks.length || user?.role !== 'user') return turn;
    const name = commandName(user.text);
    return name ? { ...turn, command: { name } } : turn;
  });
}

// Report only settings that actually changed on the ACP wire. The typed command
// alone cannot prove that a mode, model, or permission policy was applied.
export function commandChanges(before: TurnSettings, after: SessionControls) {
  const mode = before.modeId !== after.modeId && after.modeId
    ? after.modes.find(m => m.id === after.modeId)?.name ?? after.modeId : undefined;
  const options = after.options.filter(c => c.value !== before.config[c.id]).map(c => ({
    name: c.name, value: c.options.find(o => o.id === c.value)?.name ?? c.value,
  }));
  return { ...(mode ? { mode } : {}), ...(options.length ? { options } : {}) };
}
