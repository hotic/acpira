import { describe, expect, it } from 'vitest';
import { isSafeExternalUrl, type WebviewMsg } from '../src/shared/protocol';

// The host hands accepted URLs to vscode.env.openExternal, so the scheme whitelist is the whole security boundary
describe('isSafeExternalUrl', () => {
  it('accepts https / http / mailto', () => {
    expect(isSafeExternalUrl('https://example.com/x?y=1')).toBe(true);
    expect(isSafeExternalUrl('http://example.com')).toBe(true);
    expect(isSafeExternalUrl('mailto:a@b.com')).toBe(true);
  });

  it('rejects dangerous or local schemes', () => {
    expect(isSafeExternalUrl('javascript:alert(1)')).toBe(false);
    expect(isSafeExternalUrl('file:///etc/passwd')).toBe(false);
    expect(isSafeExternalUrl('vscode://evillens/command')).toBe(false);
    expect(isSafeExternalUrl('data:text/html,<script>1</script>')).toBe(false);
  });

  it('rejects non-URLs', () => {
    expect(isSafeExternalUrl('')).toBe(false);
    expect(isSafeExternalUrl('not a url')).toBe(false);
    expect(isSafeExternalUrl('/relative/path')).toBe(false);
  });
});

describe('fork / export message variants', () => {
  it('forkSession and exportSession are WebviewMsg shapes the host routes', () => {
    const fork: WebviewMsg = { type: 'forkSession', sessionId: 's1', turnIndex: 1 };
    const exp: WebviewMsg = { type: 'exportSession', id: 's1', format: 'markdown' };
    expect(fork.type).toBe('forkSession');
    expect(exp.type).toBe('exportSession');
  });

  it('native session listing / import are WebviewMsg shapes the host routes', () => {
    const list: WebviewMsg = { type: 'listNativeSessions', agent: 'opencode' };
    const imp: WebviewMsg = { type: 'importNativeSession', agent: 'opencode', sessionId: 'ses_1', cwd: '/repo', title: 'hello', updatedAt: '2026-01-01T00:00:00Z' };
    expect(list.type).toBe('listNativeSessions');
    expect(imp.type).toBe('importNativeSession');
  });
});
