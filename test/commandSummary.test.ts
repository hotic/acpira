import { afterEach, describe, expect, it } from 'vitest';
import { setLocale } from '../src/webview/i18n';
import { commandDuration, commandSummary } from '../src/webview/chat/commandSummary';

afterEach(() => setLocale('en'));

describe('command row summary', () => {
  it('keeps the program and up to two subcommand words, marking anything left out', () => {
    expect(commandSummary('pnpm typecheck')).toBe('pnpm typecheck');
    expect(commandSummary('git status')).toBe('git status');
    expect(commandSummary('pnpm test --reporter=verbose')).toBe('pnpm test …');
    expect(commandSummary('pnpm vitest run test/folding.test.ts')).toBe('pnpm vitest run …');
    expect(commandSummary('docker compose up web')).toBe('docker compose up …');
    expect(commandSummary('ls -la')).toBe('ls …');
    expect(commandSummary('pnpm typecheck >/dev/null 2>&1 && echo TYPECHECK_OK; pnpm test 2>&1 | grep -E "Test Files|Tests  "'))
      .toBe('pnpm typecheck …');
  });

  it('stops at a statement separator glued to a word', () => {
    expect(commandSummary('pnpm typecheck; pnpm test')).toBe('pnpm typecheck …');
    expect(commandSummary('ls;')).toBe('ls');
  });

  it('names the program of a heredoc and marks the script lines', () => {
    expect(commandSummary("python3 - <<'EOF'\nprint('done')\nEOF")).toBe('python3 …');
    expect(commandSummary("cat > /tmp/x.txt <<EOF\nhello\nEOF")).toBe('cat …');
  });

  it('skips a leading cd and environment assignments', () => {
    expect(commandSummary('cd rust && cargo test --workspace 2>&1 | tail -n 72')).toBe('cargo test …');
    expect(commandSummary('cd "My Project"; pnpm build')).toBe('pnpm build');
    expect(commandSummary('cd a && cd b && make')).toBe('make');
    expect(commandSummary('CI=1 NODE_ENV=test pnpm test')).toBe('pnpm test');
    expect(commandSummary(`FOO="a b" BAR='c' node script.js`)).toBe('node script.js');
    // A lone assignment is the command itself
    expect(commandSummary('FOO=1')).toBe('FOO=1');
    // Anything beyond a simple `cd dir` stays: the summary never guesses through a subshell or a variable
    expect(commandSummary('cd $(git rev-parse --show-toplevel) && pnpm test')).toBe('cd …');
  });

  it('unwraps a whole-command shell wrapper', () => {
    expect(commandSummary("bash -lc 'pnpm test --run'")).toBe('pnpm test …');
    expect(commandSummary('/bin/zsh -c "cd web && npm run build"')).toBe('npm run build');
    // Mixed quoting is left alone
    expect(commandSummary(`bash -c 'echo "x"' && echo 'y'`)).toBe('bash …');
  });

  it('reads an absolute program path by its name and keeps relative ones', () => {
    expect(commandSummary('/usr/bin/python3 -m http.server')).toBe('python3 …');
    expect(commandSummary('./gradlew test')).toBe('./gradlew test');
    expect(commandSummary('node_modules/.bin/tsx scripts/probe.ts grok')).toBe('node_modules/.bin/tsx …');
  });

  it('handles blank and padded input', () => {
    expect(commandSummary('')).toBe('');
    expect(commandSummary('   \n  ')).toBe('');
    expect(commandSummary('  git   log  \n')).toBe('git log');
  });
});

describe('command run time', () => {
  it('keeps tenths below ten seconds and whole seconds beyond', () => {
    expect(commandDuration(180)).toBe('0.2s');
    expect(commandDuration(40)).toBe('<0.1s');
    expect(commandDuration(2_400)).toBe('2.4s');
    expect(commandDuration(41_800)).toBe('42s');
    expect(commandDuration(125_000)).toBe('2m 5s');
    expect(commandDuration(120_000)).toBe('2m');
    expect(commandDuration(-5)).toBe('<0.1s');
  });

  it('ticks a live clock in whole seconds', () => {
    expect(commandDuration(5_900, true)).toBe('5s');
    expect(commandDuration(0, true)).toBe('0s');
    expect(commandDuration(61_500, true)).toBe('1m 1s');
  });

  it('follows the locale', () => {
    setLocale('zh-CN');
    expect(commandDuration(180)).toBe('0.2 秒');
    expect(commandDuration(125_000)).toBe('2 分钟 5 秒');
  });
});
