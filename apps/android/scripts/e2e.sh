#!/usr/bin/env bash
# End to end on the emulator: the app against a real engine (apps/ios/scripts/engine.sh: `acpira --ws` on a throwaway
# home whose only agent is the fake one, so no model call is spent), driven through its own screens with uiautomator.
#   1. a new chat from the start page: type, send, the fake agent's streamed reply shows
#   2. a tool turn in the same chat: the approval card shows, Allow runs it, the reply shows and the card goes
# Needs a booted emulator or device (adb devices). SKIP_BUILD=1 reuses the installed build; PORT picks the engine's.
# Usage: apps/android/scripts/e2e.sh
set -euo pipefail

repo="$(cd "$(dirname "$0")/../../.." && pwd)"
android="$repo/apps/android"
export PATH="${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools:$PATH"
port="${PORT:-7362}"
package="cc.hotic.acpira"
ui() { python3 "$android/scripts/ui.py" "$@"; }
step() { printf '\n== %s\n' "$*"; }

[ -n "$(adb devices | awk 'NR>1 && $2=="device"')" ] || { echo "no emulator or device: boot one first" >&2; exit 1; }

log="$(mktemp /tmp/acpira-android-e2e-XXXX)"
engine=""
cleanup() {
  # engine.sh takes the engine, its fake agents and its home with it on TERM
  if [ -n "$engine" ]; then kill -TERM "$engine" 2>/dev/null || true; wait "$engine" 2>/dev/null || true; fi
  adb reverse --remove "tcp:$port" >/dev/null 2>&1 || true
  rm -f "$log"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

step "engine on $port"
"$repo/apps/ios/scripts/engine.sh" "$port" >"$log" 2>&1 &
engine=$!
for _ in $(seq 120); do
  grep -q '^-engine ' "$log" && break
  kill -0 "$engine" 2>/dev/null || { cat "$log" >&2; exit 1; }
  sleep 0.5
done
address="$(sed -n 's/^-engine //p' "$log")"
[ -n "$address" ] || { cat "$log" >&2; echo "the engine printed no address" >&2; exit 1; }
# The app dials 127.0.0.1 on the device; reverse forwarding lands it on the Mac's engine
adb reverse "tcp:$port" "tcp:$port" >/dev/null

if [ "${SKIP_BUILD:-0}" != 1 ]; then
  step "build and install"
  (cd "$android" && JAVA_HOME="${JAVA_HOME:-$(/usr/libexec/java_home -v 21)}" ./gradlew :app:assembleDebug -q)
  adb install -r "$android/app/build/outputs/apk/debug/app-debug.apk" >/dev/null
  # An install relaunches the replaced activity with its old intent; let it settle before the real launch
  adb shell am force-stop "$package"
  sleep 1
fi

step "launch on the engine"
# Granted up front: the first live wire would otherwise ask for notifications over the screens the run drives
adb shell pm grant "$package" android.permission.POST_NOTIFICATIONS 2>/dev/null || true
adb shell am start -W -S -n "$package/.MainActivity" --es engine "$address" --es locale en --es motion none >/dev/null
# Connected: the engine's empty list replaces "Connecting" (agents and settings have landed with init)
ui wait "No sessions yet" 30

step "a new chat: send, the reply streams in"
ui tap list.new
ui wait composer.field
ui tap composer.field
# The keyboard has to attach to the field before keys land in it
sleep 1
ui type "ping from android"
ui tap composer.send
ui wait chat.thread 30
# The keyboard capitalises the first word: wait on the rest
ui wait "from android" 15
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
