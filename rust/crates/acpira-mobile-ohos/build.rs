//! On OpenHarmony the library links the system's N-API (`libace_napi.z.so`), found in the SDK's sysroot: `OHOS_NDK_HOME`
//! names the SDK's API-level folder (the one holding `native/`), as apps/harmony/scripts/build-net.sh sets it. Nothing
//! happens for any other target.

use std::env;

fn main() {
  println!("cargo::rerun-if-env-changed=OHOS_NDK_HOME");
  if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("ohos") {
    return;
  }
  let Ok(sdk) = env::var("OHOS_NDK_HOME") else {
    println!("cargo::error=OHOS_NDK_HOME must name the OpenHarmony SDK's API-level folder (holding native/)");
    return;
  };
  let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
    Ok("aarch64") => "aarch64-linux-ohos",
    Ok("x86_64") => "x86_64-linux-ohos",
    Ok("arm") => "arm-linux-ohos",
    other => {
      println!("cargo::error=no OpenHarmony sysroot for {other:?}");
      return;
    }
  };
  println!("cargo::rustc-link-search=native={sdk}/native/sysroot/usr/lib/{arch}");
  println!("cargo::rustc-link-lib=dylib=ace_napi.z");
}
