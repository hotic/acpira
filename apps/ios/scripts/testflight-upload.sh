#!/bin/sh
# Exports an archive as an App Store .ipa and uploads it to TestFlight without Xcode's account sign-in, then puts the
# build into the public group (testflight-public.mjs).
#
#   testflight-upload.sh <Acpira.xcarchive> [out-dir]
#
# Why not `xcodebuild -exportArchive` with automatic signing and `destination upload`: that path needs an Apple
# account signed in to Xcode, whose account list has been found empty more than once, and with an App Store Connect
# API key instead it asks for a cloud-managed distribution certificate the key has no access to. Manual signing needs
# neither:
#   - the "Apple Distribution" identity with its private key in a keychain on the user's search list
#   - the App Store provisioning profiles named below in ~/Library/MobileDevice/Provisioning Profiles
#   - the App Store Connect API key: ~/.appstoreconnect/issuer_id and the single
#     ~/.appstoreconnect/private_keys/AuthKey_<KEY_ID>.p8 (ASC_ISSUER_ID / ASC_KEY_PATH override them)
# Run it outside a sandbox: codesign reads the keychain, which must be unlocked in this login session.
set -eu

[ $# -ge 1 ] || { echo "usage: $0 <Acpira.xcarchive> [out-dir]" >&2; exit 2; }
archive=$(cd "$1" && pwd)
out=${2:-$(dirname "$archive")}
mkdir -p "$out"
out=$(cd "$out" && pwd)
here=$(cd "$(dirname "$0")" && pwd)
team=2DUQS6323W

asc="$HOME/.appstoreconnect"
key=${ASC_KEY_PATH:-$(ls "$asc"/private_keys/AuthKey_*.p8 | head -1)}
key_id=$(basename "$key" .p8 | sed 's/^AuthKey_//')
issuer=${ASC_ISSUER_ID:-$(tr -d '[:space:]' < "$asc/issuer_id")}

identity=$(security find-identity -v -p codesigning | sed -n "s/^ *[0-9]*) \\([0-9A-F]*\\) \"Apple Distribution: .*($team)\"\$/\\1/p" | head -1)
[ -n "$identity" ] || { echo "no Apple Distribution identity for $team in the keychain search list" >&2; exit 1; }
# A locked keychain leaves codesign waiting on a password dialog, and the export hangs with it: probe with a time limit.
# A keychain other than the login one is locked again in every new login session until `security unlock-keychain`
probe=$(mktemp)
cp /usr/bin/true "$probe"
codesign -f -s "$identity" "$probe" > /dev/null 2>&1 &
pid=$!
n=0
while kill -0 "$pid" 2> /dev/null && [ "$n" -lt 20 ]; do sleep 1; n=$((n + 1)); done
if kill -0 "$pid" 2> /dev/null; then
  kill "$pid"; rm -f "$probe"
  echo "codesign waits on a keychain dialog: unlock the keychain that holds the Apple Distribution key first" >&2
  exit 1
fi
wait "$pid" || { rm -f "$probe"; echo "codesign with the Apple Distribution identity failed" >&2; exit 1; }
rm -f "$probe"

build=$(/usr/libexec/PlistBuddy -c 'Print :ApplicationProperties:CFBundleVersion' "$archive/Info.plist")

# A profile mapped to a bundle id the archive does not hold is ignored, so extensions are listed ahead of their landing
options="$out/ExportOptions-manual.plist"
cat > "$options" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>method</key><string>app-store-connect</string>
  <key>destination</key><string>export</string>
  <key>teamID</key><string>$team</string>
  <key>signingStyle</key><string>manual</string>
  <key>signingCertificate</key><string>Apple Distribution</string>
  <key>manageAppVersionAndBuildNumber</key><false/>
  <key>provisioningProfiles</key>
  <dict>
    <key>cc.hotic.acpira</key><string>Acpira App Store Distribution</string>
    <key>cc.hotic.acpira.widgets</key><string>Acpira Widgets App Store Distribution</string>
  </dict>
</dict>
</plist>
EOF

# Homebrew's rsync on PATH makes the export fail with "Copy failed": the system tools only
rm -rf "$out/export" "$out/export.log" "$out/upload.log"
env PATH=/usr/bin:/bin:/usr/sbin:/sbin xcodebuild -exportArchive -archivePath "$archive" -exportPath "$out/export" \
  -exportOptionsPlist "$options" \
  -authenticationKeyPath "$key" -authenticationKeyID "$key_id" -authenticationKeyIssuerID "$issuer" > "$out/export.log" 2>&1 \
  || { tail -20 "$out/export.log" >&2; exit 1; }
ipa=$(ls "$out"/export/*.ipa | head -1)
echo "exported build $build: $ipa"

# altool finds AuthKey_<id>.p8 by its id, in the key's own folder
API_PRIVATE_KEYS_DIR=$(dirname "$key") xcrun altool --upload-app -f "$ipa" -t ios --apiKey "$key_id" --apiIssuer "$issuer" > "$out/upload.log" 2>&1 \
  || { tail -20 "$out/upload.log" >&2; exit 1; }
echo "uploaded build $build"

node "$here/testflight-public.mjs" "$build"
