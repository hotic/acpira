#!/usr/bin/env bash
# A paired-remote gateway for the app and AcpiraRemote's integration test: `acpira-gateway run` on a throwaway home and
# HOME with relays off (LAN only), bridged to an `acpira serve` whose only enabled agent is the fake agent
# (test/fake-agent.ts), so nothing spends a model call, reads a real login or touches ~/.acpira. It keeps a pairing
# window open (one pairing each) and the window's link in a file whose path it prints, and serves until stopped; the
# home, the engine and its agents go with it.
#   ACPIRA_TEST_GATEWAY=<that file> swift test --filter AcpiraRemoteTests    (in Packages/AcpiraKit)
# Usage: apps/ios/scripts/gateway.sh [--test]   (--test: run that test against it once, then stop)
set -euo pipefail

ios="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$ios/../.." && pwd)"
bin="$repo/rust/target/debug"
fake="$repo/node_modules/.cache/acpira-test/fake-agent.mjs"

(cd "$repo/rust" && cargo build --quiet --bin acpira --bin acpira-gateway)
if [ ! -f "$fake" ]; then
  (cd "$repo" && pnpm exec tsx -e "import('./test/fakeAgentBundle.ts').then(m => m.bundleFakeAgent())")
fi

# Short: the gateway's and the engine's sockets live under it
root="$(mktemp -d /tmp/acpira-gw-XXXX)"
home="$root/acpira"
mkdir -p "$home"
gw=""
cleanup() {
  # The gateway, its `pair` windows and the engine it started all name the home on their command line; the engine
  # ends its agents when it is stopped
  [ -n "$gw" ] && kill "$gw" 2>/dev/null
  pkill -f -- "$home" 2>/dev/null || true
  wait 2>/dev/null || true
  rm -rf "$root"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# Every built-in agent off, so a tap on one cannot start a real CLI (as engine.sh)
disabled="$(env HOME="$root" "$bin/acpira" agents --json --home "$home" 2>/dev/null \
  | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.stringify(JSON.parse(s).agents.map(a=>a.id))))')"
printf '{"fake":{"name":"Fake agent","command":"%s","args":["%s"]}}\n' "$(command -v node)" "$fake" > "$home/agents.json"
printf '{"defaultAgent":"fake","disabledAgents":%s}\n' "$disabled" > "$home/settings.json"
mkdir -p "$root/project"

gateway() { env HOME="$root" ACPIRA_CATALOG_REFRESH=0 "$bin/acpira-gateway" "$@" --home "$home"; }
gateway run --engine "$bin/acpira" --relays off 2> "$root/gateway.log" &
gw=$!
until [ -S "$home/run/gateway.sock" ]; do
  kill -0 "$gw" 2>/dev/null || { cat "$root/gateway.log" >&2; exit 1; }
  sleep 0.2
done

# A fresh window whenever the last one was used or ran out; `pair` itself waits for the pairing and then exits
link="$root/link"
keep_window() {
  while kill -0 "$gw" 2>/dev/null; do
    now="$(gateway status --json 2>/dev/null | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{try{console.log(JSON.parse(s).pairing?.link??"")}catch{console.log("")}})')"
    if [ -z "$now" ]; then
      gateway pair >/dev/null 2>&1 &
    else
      printf '%s\n' "$now" > "$link.tmp" && mv "$link.tmp" "$link"
    fi
    sleep 0.5
  done
}
keep_window &
until [ -s "$link" ]; do sleep 0.2; done

echo "gateway on a temporary home ($home), log $root/gateway.log, a project at $root/project" >&2
echo "ACPIRA_TEST_GATEWAY=$link"

if [ "${1:-}" = "--test" ]; then
  stamp="$(cat "$ios/Packages/AcpiraKit/Artifacts/stamp" 2>/dev/null || echo none)"
  (cd "$ios/Packages/AcpiraKit" && ACPIRA_TEST_GATEWAY="$link" ACPIRA_NET_STAMP="$stamp" swift test --filter AcpiraRemoteTests)
  exit
fi
wait "$gw"
