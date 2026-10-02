import { describe, expect, it } from 'vitest';
import { legacyAgents } from '../src/host/legacyAgents';

describe('legacy agent migration boundary', () => {
  const windows = { devin: { command: 'C:\\Users\\Spark\\AppData\\Local\\devin\\cli\\bin\\devin.exe', args: ['acp'] } };
  it('does not send inherited client commands or env to a remote host, even on the same OS', () => {
    expect(legacyAgents(windows, 'ssh-remote')).toBeUndefined();
    expect(legacyAgents({ devin: { command: '/home/client/bin/devin', env: { PRIVATE: 'client-only' } } }, 'ssh-remote')).toBeUndefined();
    expect(legacyAgents(windows, 'wsl')).toBeUndefined();
    expect(legacyAgents(windows, 'dev-container')).toBeUndefined();
  });
  it('retains definitions for the one-time migration on the local extension host', () => {
    expect(legacyAgents(windows, undefined)).toEqual(windows);
  });
});
