#!/usr/bin/env bash
# Signs the debug HAP with the OpenHarmony test keys the public SDK ships (toolchains/lib: OpenHarmony.p12, password
# 123456), for an OpenHarmony emulator or a developer-mode board. A Huawei HarmonyOS device needs AppGallery Connect
# certificates instead. The profile is the SDK's unsigned debug template with this app's bundle name and a fresh validity.
# Usage: apps/harmony/scripts/sign.sh   → entry/build/default/outputs/default/entry-default-signed.hap
set -euo pipefail
here="$(cd "$(dirname "$0")/.." && pwd)"
sdk="${OHOS_SDK:-$HOME/Library/OpenHarmony/sdk}"
lib="$(ls -d "$sdk"/*/toolchains/lib | sort -V | tail -1)"
out="$here/entry/build/default/outputs/default"
work="$here/build/sign" && rm -rf "$work" && mkdir -p "$work"
tool=(java -jar "$lib/hap-sign-tool.jar")

# The application certificate the template names (issued by the Application CA; the keystore's own entry for the same
# key is self-signed) and that CA, as one chain, leaf first
openssl pkcs12 -in "$lib/OpenHarmony.p12" -passin pass:123456 -nokeys -legacy 2>/dev/null > "$work/all.pem" \
  || openssl pkcs12 -in "$lib/OpenHarmony.p12" -passin pass:123456 -nokeys > "$work/all.pem"
python3 - "$lib/UnsgnedDebugProfileTemplate.json" "$work/all.pem" "$work/app.cer" <<'PY'
import json, re, sys
leaf = json.load(open(sys.argv[1]))['bundle-info']['development-certificate'].strip()
text = open(sys.argv[2]).read()
ca = next(pem for name, pem in re.findall(r'friendlyName: ([^\n]*)\n.*?(-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----)', text, re.S)
          if name.strip().lower() == 'openharmony application ca')
open(sys.argv[3], 'w').write(leaf + '\n' + ca + '\n')
PY

python3 - "$lib/UnsgnedDebugProfileTemplate.json" "$work/profile.json" <<'PY'
import json, sys, time, uuid
p = json.load(open(sys.argv[1]))
p['uuid'] = str(uuid.uuid4())
now = int(time.time())
p['validity'] = {'not-before': now - 86400, 'not-after': now + 365 * 86400}
p['bundle-info']['bundle-name'] = 'cc.hotic.acpira'
# Any device: the emulator's id is not known up front
p.pop('debug-info', None)
json.dump(p, open(sys.argv[2], 'w'), indent=2)
PY

"${tool[@]}" sign-profile -mode localSign -keyAlias "openharmony application profile debug" -keyPwd 123456 \
  -profileCertFile "$lib/OpenHarmonyProfileDebug.pem" -inFile "$work/profile.json" -signAlg SHA256withECDSA \
  -keystoreFile "$lib/OpenHarmony.p12" -keystorePwd 123456 -outFile "$work/profile.p7b"
"${tool[@]}" sign-app -mode localSign -keyAlias "openharmony application release" -keyPwd 123456 \
  -appCertFile "$work/app.cer" -profileFile "$work/profile.p7b" -inFile "$out/entry-default-unsigned.hap" \
  -signAlg SHA256withECDSA -keystoreFile "$lib/OpenHarmony.p12" -keystorePwd 123456 \
  -outFile "$out/entry-default-signed.hap"
echo "signed: $out/entry-default-signed.hap"
