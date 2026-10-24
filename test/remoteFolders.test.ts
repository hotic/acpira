import { describe, expect, it } from 'vitest';
import { formatRemoteFolder, isRemoteFolder, parseRemoteFolder } from '@acpira/shared/remote';
import fixture from './fixtures/remote-folders.json';

// The same fixture runs through acpira_shared::remote (tests::parses_exactly_like_the_typescript_mirror)
describe('remote folders', () => {
  it.each(fixture)('parse $input like the Rust side', ({ input, alias, path, canonical }) => {
    const parsed = parseRemoteFolder(input);
    expect(parsed?.alias ?? null).toBe(alias);
    expect(parsed?.path ?? null).toBe(path);
    expect((parsed && formatRemoteFolder(parsed.alias, parsed.path)) ?? null).toBe(canonical);
  });

  it('never takes a local path for a remote one', () => {
    expect(isRemoteFolder('/ssh:/x')).toBe(false);
    expect(formatRemoteFolder('-x', '/a')).toBeUndefined();
    expect(formatRemoteFolder('gen1', 'relative')).toBeUndefined();
  });
});
