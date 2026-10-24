import { describe, expect, it } from 'vitest';
import { parseEnvOutput } from '../src/loginEnv';

const MARK = '__ACPIRA_ENV_7f3c__';

describe('parseEnvOutput', () => {
  it('reads the pairs between the marks, keeps values with = and newlines, drops per-process ones', () => {
    const body = ['PATH=/opt/homebrew/bin:/usr/bin', 'OPENAI_API_KEY=sk-x=y', 'MULTI=a\nb', 'PWD=/tmp', 'SHLVL=2', '_=/usr/bin/env', 'ELECTRON_RUN_AS_NODE=1', 'BROKEN', ''].join('\0');
    const out = `rc banner\n${MARK}${body}${MARK}trailing`;
    expect(parseEnvOutput(out)).toEqual({ PATH: '/opt/homebrew/bin:/usr/bin', OPENAI_API_KEY: 'sk-x=y', MULTI: 'a\nb' });
  });

  it('gives up without both marks', () => {
    expect(parseEnvOutput('PATH=/usr/bin')).toBeUndefined();
    expect(parseEnvOutput(`${MARK}PATH=/usr/bin`)).toBeUndefined();
  });
});
