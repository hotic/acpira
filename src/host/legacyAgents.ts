// Remote configuration is merged with the client User settings by VS Code. It cannot identify
// which host supplied each command / env entry, so it must never seed the remote machine's store.
export function legacyAgents(value: unknown, remoteName: string | undefined): unknown {
  return remoteName ? undefined : value;
}
