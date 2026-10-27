#!/usr/bin/env bash
# A development engine for the app: `acpira --ws` (the harness wire) on a throwaway home and HOME whose only enabled
# agent is the fake agent (test/fake-agent.ts), so nothing the app does spends a model call or reads a real login.
# Prints the app's launch argument (`-engine PORT:TOKEN`) and serves until stopped; the home goes with it.
# The simulator reaches the Mac's loopback; a device does not (that is the remote module's connection).
# Usage: apps/ios/scripts/engine.sh [port]
set -euo pipefail

repo="$(cd "$(dirname "$0")/../../.." && pwd)"
port="${1:-7360}"
bin="$repo/rust/target/debug/acpira"
fake="$repo/node_modules/.cache/acpira-test/fake-agent.mjs"

[ -x "$bin" ] || { echo "no $bin: run pnpm test once, or cargo build --bin acpira in rust/" >&2; exit 1; }
if [ ! -f "$fake" ]; then
  (cd "$repo" && pnpm exec tsx -e "import('./test/fakeAgentBundle.ts').then(m => m.bundleFakeAgent())")
fi

root="$(mktemp -d /tmp/acpira-ios-engine-XXXX)"
pid=""
# The engine (and the fake agents it started) go with the script, however it is stopped
trap '[ -n "$pid" ] && kill "$pid" 2>/dev/null; wait 2>/dev/null; rm -rf "$root"' EXIT
trap 'exit 130' INT TERM
home="$root/acpira"
mkdir -p "$home"
# Every built-in agent off, so a tap on one cannot start a real CLI
disabled="$(env HOME="$root" "$bin" agents --json --home "$home" 2>/dev/null \
  | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.stringify(JSON.parse(s).agents.map(a=>a.id))))')"
printf '{"fake":{"name":"Fake agent","command":"%s","args":["%s"]}}\n' "$(command -v node)" "$fake" > "$home/agents.json"
printf '{"defaultAgent":"fake","disabledAgents":%s}\n' "$disabled" > "$home/settings.json"
mkdir -p "$root/project"

token="$(openssl rand -hex 16)"
echo "engine on 127.0.0.1:$port, home $home, a project at $root/project" >&2
echo "-engine $port:$token"
# The harness serves its pages from the working directory; the app only needs /ws and /blobs
cd "$repo"
env HOME="$root" "$bin" --ws "$port" --home "$home" --token "$token" &
pid=$!
wait "$pid"
