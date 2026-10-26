//! Packs the exported i18n dictionaries (i18n/*.json) DEFLATE-compressed into OUT_DIR, where src/i18n.rs embeds them.
//! The nine JSON files are ~890 KB and both the engine and the desktop shell link this crate; packed they are ~240 KB

#![allow(clippy::expect_used, clippy::panic, reason = "a build script fails the build by panicking")]

use std::path::Path;
use std::{env, fs};

// Same order as `Locale` in src/i18n.rs
const TAGS: [&str; 9] = ["zh-CN", "zh-TW", "en", "ja", "ko", "es", "de", "fr", "ru"];

fn main() {
  let out = env::var("OUT_DIR").expect("OUT_DIR");
  for tag in TAGS {
    let src = format!("i18n/{tag}.json");
    println!("cargo:rerun-if-changed={src}");
    let json = fs::read(&src).unwrap_or_else(|e| panic!("{src}: {e}"));
    let packed = miniz_oxide::deflate::compress_to_vec(&json, 9);
    fs::write(Path::new(&out).join(format!("{tag}.json.deflate")), packed).unwrap_or_else(|e| panic!("{tag}: {e}"));
  }
}
