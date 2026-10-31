#!/usr/bin/env bash
# Imports the Developer ID Application certificate (APPLE_CERTIFICATE, a base64 .p12, and APPLE_CERTIFICATE_PASSWORD)
# into a temporary keychain on a macOS runner and exports its identity as APPLE_SIGNING_IDENTITY through $GITHUB_ENV.
set -euo pipefail
keychain="$RUNNER_TEMP/acpira.keychain-db"
pass=$(openssl rand -hex 16)
echo "$APPLE_CERTIFICATE" | base64 --decode > "$RUNNER_TEMP/cert.p12"
security create-keychain -p "$pass" "$keychain"
security set-keychain-settings -lut 3600 "$keychain"
security unlock-keychain -p "$pass" "$keychain"
security import "$RUNNER_TEMP/cert.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple: -k "$pass" "$keychain" > /dev/null
security list-keychains -d user -s "$keychain" $(security list-keychains -d user | tr -d '"')
rm "$RUNNER_TEMP/cert.p12"
identity=$(security find-identity -v -p codesigning "$keychain" | grep -o '"Developer ID Application: [^"]*"' | head -1 | tr -d '"')
test -n "$identity"
echo "APPLE_SIGNING_IDENTITY=$identity" >> "$GITHUB_ENV"
