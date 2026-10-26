use std::path::{Path, PathBuf};

fn main() {
  embed_page();
  tauri_build::build();
}

/// Windows ships one exe, so the page bundle and the picker script go inside it (src/embedded.rs serves them), laid out
/// as the macOS package lays out its resources (scripts/package.mjs): files under assets/ zstd-compressed, main.js and
/// main.css plain, so the updater's patch from one exe to the next stays small when only those change. Other targets,
/// and a checkout without a page build in dist/, get an empty table and read the page from disk
fn embed_page() {
  let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
  let dist = Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../../dist");
  println!("cargo:rerun-if-changed=../../dist/webview");
  println!("cargo:rerun-if-changed=../../dist/picker.js");
  println!("cargo:rerun-if-env-changed=ACPIRA_EMBED_PAGE");
  let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
  let wanted = windows && std::env::var("ACPIRA_EMBED_PAGE").as_deref() != Ok("0");
  let webview = dist.join("webview");
  let mut entries: Vec<(String, PathBuf, bool)> = vec![];
  let mut picker = None;
  if wanted && webview.join("main.js").is_file() {
    // Level 19 for a release build; a dev build's build script runs unoptimized, where 19 takes far longer
    let level = if std::env::var("PROFILE").as_deref() == Ok("release") { 19 } else { 3 };
    let staged = out.join("page");
    let _ = std::fs::remove_dir_all(&staged);
    let mut files = vec![];
    walk(&webview, &mut files);
    files.sort();
    for file in files {
      let rel = file.strip_prefix(&webview).expect("under the bundle").to_string_lossy().replace('\\', "/");
      let packed = rel.starts_with("assets/");
      let dest = staged.join(if packed { format!("{rel}.zst") } else { rel.clone() });
      std::fs::create_dir_all(dest.parent().expect("a parent")).expect("staging directory");
      let bytes = std::fs::read(&file).expect("a bundle file");
      let body = if packed { zstd::encode_all(bytes.as_slice(), level).expect("zstd") } else { bytes };
      std::fs::write(&dest, body).expect("staged file");
      entries.push((rel, dest, packed));
    }
    if dist.join("picker.js").is_file() {
      let dest = out.join("picker.js");
      std::fs::copy(dist.join("picker.js"), &dest).expect("picker");
      picker = Some(dest);
    }
  } else if wanted {
    // A package build (ACPIRA_EMBED_PAGE=1, scripts/package-windows.mjs) must not ship an exe without its page
    assert!(std::env::var("ACPIRA_EMBED_PAGE").as_deref() != Ok("1"), "no page build in dist/webview to embed: run pnpm build first");
    println!("cargo:warning=no page build in dist/webview: this exe serves the page from disk (pnpm build first to embed it)");
  }
  let mut code = String::from("/// The page bundle inside the exe: (path under webview/, bytes, zstd-compressed)\npub static WEBVIEW: &[(&str, &[u8], bool)] = &[\n");
  for (rel, path, packed) in &entries {
    code.push_str(&format!("  ({rel:?}, include_bytes!({:?}), {packed}),\n", path.to_string_lossy()));
  }
  code.push_str("];\n/// The element picker the preview proxy injects\npub static PICKER: Option<&[u8]> = ");
  match &picker {
    Some(p) => code.push_str(&format!("Some(include_bytes!({:?}));\n", p.to_string_lossy())),
    None => code.push_str("None;\n"),
  }
  std::fs::write(out.join("embedded_page.rs"), code).expect("embedded_page.rs");
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
  for e in std::fs::read_dir(dir).expect("readable bundle").flatten() {
    let p = e.path();
    if p.is_dir() {
      walk(&p, files);
    } else {
      files.push(p);
    }
  }
}
