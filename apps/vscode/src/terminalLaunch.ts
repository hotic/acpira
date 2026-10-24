// Windows requests already contain the engine's prepared shell launch.
// POSIX requests are delivered as quoted text to the user's terminal.
export function terminalLaunch(command: string, args: string[], platform = process.platform): {
  shellPath?: string;
  shellArgs?: string[];
  text?: string;
} {
  if (platform === 'win32') {
    return { shellPath: command, shellArgs: args };
  }
  return { text: [command, ...args].map(shellQuote).join(' ') };
}

function shellQuote(s: string): string {
  return /^[\w./=:@%+-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`;
}
