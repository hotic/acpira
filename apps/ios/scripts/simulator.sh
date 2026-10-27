# Sourced by the iOS scripts, not run: one simulator run at a time, and no simulator left booted for good.
#
# Parallel sessions share one device. A lock serialises the scripts (build, install, launch, capture), so one
# session's launch never kills another's app mid-shot and nobody needs a device of their own. A device a script
# booted is shut down once no script has used it for ACPIRA_SIM_IDLE seconds (default 600; 0 keeps it booted),
# so a few shots in a row reuse the booted device; a device booted by hand is left alone.
#
# The scripts default to a device of their own (`sim_default`, an iPhone 18 Pro named "Acpira Scripts", created on
# first use), never the plain "iPhone 18 Pro" a person keeps open to try the app: a shot reinstalls the app and
# relaunches it with its own arguments (`-motion none` turns every animation off), and the reaper above shuts down a
# device the scripts booted. A simulator named on the command line is still used as given.

sim_state="/tmp/acpira-ios-sim-$(id -u)"
sim_lock="$sim_state/lock"
sim_default="Acpira Scripts"
sim_default_type="com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro"

# True when a device of exactly this name exists (any state)
sim_exists() {
  xcrun simctl list devices |
    awk -v n="$1 (" '{ sub(/^ +/, "") } index($0, n) == 1 { f = 1 } END { exit !f }'
}

# True when the named device is booted; exact name, so "iPhone 18 Pro" is not "iPhone 18 Pro Max"
sim_is_booted() {
  xcrun simctl list devices booted |
    awk -v n="$1 (" '{ sub(/^ +/, "") } index($0, n) == 1 { f = 1 } END { exit !f }'
}

# Creates the scripts' own device on first use (newest iOS runtime); other names must already exist
sim_ensure() {
  [ "$1" = "$sim_default" ] || return 0
  sim_exists "$1" || xcrun simctl create "$1" "$sim_default_type" >/dev/null
}

sim_key() { printf '%s' "$1" | tr -c 'A-Za-z0-9' '_'; }

# Holds the lock until this script exits; scripts it runs (render.sh -> shot.sh) inherit it
sim_acquire() {
  sim_name="$1"
  [ -n "${ACPIRA_SIM_LOCK_HELD:-}" ] && return 0
  mkdir -p "$sim_state"
  local waited=0 holder
  until mkdir "$sim_lock" 2>/dev/null; do
    holder="$(cat "$sim_lock/pid" 2>/dev/null || true)"
    # A holder that died without releasing; an empty pid is a holder still writing it
    if [ -n "$holder" ] && ! kill -0 "$holder" 2>/dev/null; then
      rm -rf "$sim_lock"
      continue
    fi
    [ $((waited % 30)) -eq 0 ] && echo "waiting for the iOS simulator, held by pid ${holder:-?}" >&2
    sleep 1
    waited=$((waited + 1))
  done
  echo $$ >"$sim_lock/pid"
  export ACPIRA_SIM_LOCK_HELD=1
  sim_ensure "$sim_name"
  # Only a device this run boots is ours to shut down later
  sim_is_booted "$sim_name" || touch "$sim_state/$(sim_key "$sim_name").ours"
  trap sim_release EXIT
  trap 'exit 130' INT TERM
}

sim_release() {
  local key idle="${ACPIRA_SIM_IDLE:-600}"
  key="$(sim_key "$sim_name")"
  touch "$sim_state/$key.used"
  rm -rf "$sim_lock"
  { [ "$idle" -gt 0 ] && [ -e "$sim_state/$key.ours" ]; } || return 0
  # Detached reaper: after the idle time it takes the lock and shuts the device down,
  # unless a later run touched the stamp (that run's own reaper takes over)
  nohup bash -c '
    sleep "$1"
    mkdir "$2/lock" 2>/dev/null || exit 0
    echo $$ >"$2/lock/pid"
    if [ $(( $(date +%s) - $(stat -f %m "$2/$3.used") )) -ge "$1" ]; then
      xcrun simctl shutdown "$4" >/dev/null 2>&1
      rm -f "$2/$3.ours"
    fi
    rm -rf "$2/lock"
  ' _ "$idle" "$sim_state" "$key" "$sim_name" >/dev/null 2>&1 </dev/null &
}
