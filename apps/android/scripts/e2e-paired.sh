#!/usr/bin/env bash
# End to end over paired remote access on the emulator: the app against a real gateway (apps/ios/scripts/gateway.sh:
# `acpira serve --gateway` with relays off on a throwaway home, an engine whose only agent is the fake one), driven
# through its own screens with uiautomator.
#   1. a fresh install opens on the connect page; Add computer, the computer nearby, its 6-digit code: paired
#   2. the wire comes up and the list follows the computer
#   3. a new chat's reply streams in, then a tool turn is approved and runs
# The emulator reaches neither the computer's LAN nor its Bonjour, only its loopback as 10.0.2.2, so the debug build's
# `nearby` launch extra lists the gateway there (LaunchOptions.kt) and the code comes from `acpira gateway status`.
# Needs a booted emulator; SKIP_BUILD=1 reuses the installed build. Clears the app's data (its pairing state).
# Usage: apps/android/scripts/e2e-paired.sh
set -euo pipefail

repo="$(cd "$(dirname "$0")/../../.." && pwd)"
android="$repo/apps/android"
export PATH="${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools:$PATH"
package="cc.hotic.acpira"
ui() { python3 "$android/scripts/ui.py" "$@"; }
step() { printf '\n== %s\n' "$*"; }

[ -n "$(adb devices | awk 'NR>1 && $2=="device"')" ] || { echo "no emulator or device: boot one first" >&2; exit 1; }

log="$(mktemp /tmp/acpira-android-e2e-XXXX)"
gateway=""
cleanup() {
  # gateway.sh takes the gateway, its engine, the fake agents and the home with it on TERM
  if [ -n "$gateway" ]; then kill -TERM "$gateway" 2>/dev/null || true; wait "$gateway" 2>/dev/null || true; fi
  rm -f "$log"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

step "gateway"
"$repo/apps/ios/scripts/gateway.sh" >"$log" 2>&1 &
gateway=$!
for _ in $(seq 600); do
  grep -q '^ACPIRA_TEST_GATEWAY=' "$log" && break
  kill -0 "$gateway" 2>/dev/null || { cat "$log" >&2; exit 1; }
  sleep 0.5
done
root="$(dirname "$(sed -n 's/^ACPIRA_TEST_GATEWAY=//p' "$log")")"
[ -d "$root" ] || { cat "$log" >&2; echo "the gateway printed no home" >&2; exit 1; }
status() { env HOME="$root" "$repo/rust/target/debug/acpira" gateway status --json --home "$root/acpira"; }
id="$(status | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')"
# The gateway's IPv4 port, which the emulator reaches as 10.0.2.2
pid="$(pgrep -f "acpira serve .*--home $root/acpira --gateway" | head -1)"
port="$(lsof -nP -a -p "$pid" -i 4UDP | awk 'NR>1 { sub(/.*:/, "", $9); print $9; exit }')"
[ -n "$id" ] && [ -n "$port" ] || { echo "no gateway id or port" >&2; exit 1; }

if [ "${SKIP_BUILD:-0}" != 1 ]; then
  step "build and install"
  "$android/scripts/build-net.sh" --if-stale
  (cd "$android" && JAVA_HOME="${JAVA_HOME:-$(/usr/libexec/java_home -v 21)}" ./gradlew :app:assembleDebug -q)
  adb install -r "$android/app/build/outputs/apk/debug/app-debug.apk" >/dev/null
fi
# A fresh start: no paired computer, and the prompts a person answers once granted up front
adb shell pm clear "$package" >/dev/null
for p in CAMERA ACCESS_LOCAL_NETWORK; do adb shell pm grant "$package" "android.permission.$p" 2>/dev/null || true; done
# The cleared app's old task is torn down a beat later, and that teardown kills a process started meanwhile, leaving
# the splash up for good (and `am start -W` waiting on it); the screens' own waits stand in for -W
sleep 2

step "pair by code"
adb shell am start -S -n "$package/.MainActivity" --es nearby "$id@10.0.2.2:$port" --es locale en --es motion none >/dev/null
ui wait "No computer connected" 30
ui tap pair.action
ui tap "10.0.2.2:$port"
# The digits' field is painted clear under the boxes, so uiautomator does not list it; it has focus once the page is in
ui wait "Enter pairing code"
sleep 1
# Codes turn over; read the current one just before typing it
adb shell input text "$(status | python3 -c 'import json,sys; print(json.load(sys.stdin)["pairing"]["code"])')"
ui wait pair.name 30

step "the wire comes up"
# The first dial tries the computer's LAN addresses, which the emulator cannot reach, before 10.0.2.2
ui wait list.new 60

step "a new chat over the link"
ui tap list.new
ui wait composer.field
ui tap composer.field
sleep 1
ui type "ping over the link"
ui tap composer.send
# The keyboard capitalises the first word
ui wait "over the link" 15
ui wait "hello world" 30

step "a tool turn: approve, it runs"
ui tap composer.field
sleep 1
ui type "run the tool"
ui tap composer.send
ui wait permission.allow 30
ui tap permission.allow
ui gone permission 30
ui wait "tests passed" 30

step "passed"
