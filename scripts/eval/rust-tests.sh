#!/usr/bin/env bash
# Hidden tests of a Rust task, run from the work tree: `rust-tests.sh <crate>`. The task's hidden tests are the crate's
# test files as the reference change left them (`.eval_hidden/rust/crates/<crate>/tests/`, support modules included,
# run.ts `hidden`); they replace the work
# tree's copies, then each of them runs as its own target under a watchdog, because a failing test can leave its tokio
# runtime waiting forever on a mock reply it held. Prints one cargo-style `test result:` line per target (run.ts sums
# them) after that target's test lines; a target cut off counts every test that did not report ok as failed.
set -u
crate=$1
# Every target of the built-in agent's suite takes a few seconds when it passes
limit=${RUST_TEST_LIMIT_S:-60}
tests=rust/crates/$crate/tests
cp -R ".eval_hidden/$tests/." "$tests/"
targets=$(cd ".eval_hidden/$tests" && ls ./*.rs | sed 's|^\./||; s|\.rs$||')
args=()
for t in $targets; do args+=(--test "$t"); done
cd rust || exit 1
if ! cargo test -p "$crate" "${args[@]}" --no-run --quiet 2>../.eval_build.txt; then
  tail -60 ../.eval_build.txt
  echo "the hidden tests do not build"
  exit 1
fi
status=0
set -m
for t in $targets; do
  listed=$(cargo test -p "$crate" --test "$t" --quiet -- --list 2>/dev/null | grep -c ': test$')
  cargo test -p "$crate" --test "$t" > "../.eval_$t.txt" 2>&1 &
  pid=$!
  # The test binary is in cargo's process group: a watchdog firing ends both
  ( sleep "$limit"; kill -TERM -- -"$pid" 2>/dev/null; sleep 2; kill -KILL -- -"$pid" 2>/dev/null ) &
  dog=$!
  wait "$pid"
  code=$?
  pkill -P "$dog" 2>/dev/null
  kill "$dog" 2>/dev/null
  wait "$dog" 2>/dev/null
  ok=$(grep -cE '^test .* \.\.\. ok$' "../.eval_$t.txt")
  # An ignored test neither passes nor fails
  ignored=$(grep -cE '^test .* \.\.\. ignored' "../.eval_$t.txt")
  listed=$((listed - ignored))
  echo "== $t"
  grep -E '^test |panicked|assertion|left:|right:' "../.eval_$t.txt" | grep -v '^test result:' | head -60
  if [ "$code" -eq 0 ]; then
    echo "test result: ok. $ok passed; $((listed - ok)) failed;"
  else
    [ "$code" -gt 128 ] && echo "($t was stopped after ${limit}s)"
    echo "test result: FAILED. $ok passed; $((listed - ok)) failed;"
    status=1
  fi
done
exit "$status"
