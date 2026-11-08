//! acpira-mobile-ohos: the phone's side of paired remote access (rust/crates/acpira-mobile) exported to ArkTS through
//! N-API, for the HarmonyOS app (apps/harmony, `remote/`). It mirrors the UniFFI surface the iOS and Android apps call:
//! a node from the identity key, pairing by link or code, and a link carrying envelope lines, blobs, probes and path
//! changes. apps/harmony/scripts/build-net.sh builds it as `libacpira_mobile_ohos.so` per ABI; ArkTS imports it by that
//! name and its types come from apps/harmony/entry/src/main/cpp/types/libacpira_mobile_ohos/index.d.ts, kept by hand in
//! step with `napi.rs`.

pub mod bridge;

#[cfg(target_env = "ohos")]
pub mod napi;
