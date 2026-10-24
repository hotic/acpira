import { describe, expect, it } from 'vitest';
import { sessionIdOfLink, sessionLink, sessionRefId } from '../packages/shared/sessionLinks';
import fixture from './fixtures/session-links.json';

// The same fixture runs through acpira_shared::session_links (tests::parses_exactly_like_the_typescript_mirror)
describe('session links', () => {
  it.each(fixture)('parse $input like the Rust side', ({ input, link, ref }) => {
    expect(sessionIdOfLink(input) ?? null).toBe(link);
    expect(sessionRefId(input) ?? null).toBe(ref);
  });

  it('round-trip through both the canonical prefix and an injected base', () => {
    expect(sessionIdOfLink(sessionLink('abc'))).toBe('abc');
    expect(sessionIdOfLink(sessionLink('abc', 'cursor://hotic.acpira/session/'))).toBe('abc');
  });
});
