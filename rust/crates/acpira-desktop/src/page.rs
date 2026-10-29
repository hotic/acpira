//! What the app protocol serves (acpira-app://localhost): the HTML around the webview bundle, the theme sheet, the
//! bundle's files, attachment blobs and the local media files the tool panel previews. Mirrors the VS Code shell's page (apps/vscode/src/bridge.ts html()): the same CSP
//! shape with the app protocol as the resource source, the same inline config the bundle reads before init
//! (window.__acpira), plus `shell: 'desktop'` for the desktop-only branches

use std::path::{Path, PathBuf};

use serde_json::json;

/// The app protocol's origin as the page sees it. WKWebView serves a custom scheme as `<scheme>://localhost` in Tauri's
/// convention; WebView2 has no custom schemes, so wry serves it as `http://<scheme>.localhost` there (a secure context,
/// being under .localhost). The protocol handler sees `acpira-app://localhost` on both: wry maps the request back
pub const fn app_origin(windows: bool) -> &'static str {
  if windows { "http://acpira-app.localhost" } else { "acpira-app://localhost" }
}

pub const APP_ORIGIN: &str = app_origin(cfg!(windows));

/// The page's platform, for its window chrome (`window.__acpira.platform`): `mac`, `win` or `linux`
pub const PLATFORM: &str = if cfg!(target_os = "macos") {
  "mac"
} else if cfg!(windows) {
  "win"
} else {
  "linux"
};

/// The CSP source that admits the app's own resources: the scheme on macOS, the origin where it is plain http
fn csp_source(origin: &str) -> String {
  if origin.starts_with("http") { origin.to_owned() } else { format!("{}:", origin.split(':').next().unwrap_or(origin)) }
}

/// Whether a URL is on the app's own origin (the navigation allowlist): same scheme, host and port
pub fn is_app_url(url: &str, origin: &str) -> bool {
  match (url::Url::parse(url), url::Url::parse(origin)) {
    (Ok(u), Ok(o)) => u.scheme() == o.scheme() && u.host_str() == o.host_str() && u.port_or_known_default() == o.port_or_known_default(),
    _ => false,
  }
}
pub const SESSION_LINK_BASE: &str = "acpira://session/";

/// What VS Code provides around a webview: the --vscode-* roles tokens.css reads and the scheme class on body. Values
/// follow VS Code's default dark / light themes (the harness's test/host-preview/host.css)
pub const THEME_CSS: &str = r#"html, body, #root { margin: 0; padding: 0; height: 100%; overflow: hidden; }
body {
  --vscode-foreground: #cccccc;
  --vscode-sideBar-background: #181818;
  --vscode-editor-background: #1f1f1f;
  --vscode-widget-border: #313131;
  --vscode-focusBorder: #0078d4;
  --vscode-editor-font-family: "SF Mono", Menlo, Monaco, Consolas, monospace;
  background: var(--vscode-sideBar-background);
  color: var(--vscode-foreground);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
}
body.vscode-light {
  --vscode-foreground: #3b3b3b;
  --vscode-sideBar-background: #f8f8f8;
  --vscode-editor-background: #ffffff;
  --vscode-widget-border: #d4d4d4;
  --vscode-focusBorder: #005fb8;
}
.acp-shell :focus, .acp-shell :focus-visible { outline: none !important; }
"#;

/// The document a plugin's interface piece runs in (webview/panel/PluginFrame.tsx), framed with `sandbox="allow-scripts"`
/// and no allow-same-origin, so its origin is opaque. Its policy allows inline code and nothing from the network; the
/// page sends the piece's HTML, which replaces this document and keeps the `acpira` object: `call(method, params)` reaches
/// the plugin's own methods, `on(fn)` hears what its runtime posts (`ui/post`), `onState(fn)` hears whether the session on
/// screen is working or waiting for the user (nothing of its content), `options` holds what the page set for this frame
/// (a backdrop's cell size and frame cap). Not a boundary against the plugin itself,
/// whose code runs with the user's rights anyway; it keeps the page's state and the session out of the piece's reach
pub const PLUGIN_FRAME_HTML: &str = r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:">
</head>
<body>
<script>
(function () {
  var waits = {}, hearers = [], watchers = [], readers = [], next = 0;
  // The page's tokens on the root, first in the head so the piece's own styles win
  function style(css) {
    var el = document.getElementById('acpira-theme');
    if (!el) { el = document.createElement('style'); el.id = 'acpira-theme'; document.head.insertBefore(el, document.head.firstChild); }
    el.textContent = ':root{' + (css || '') + '}html,body{margin:0;background:transparent;color:var(--fg-1);font-family:var(--font);font-size:var(--ui-font-size)}';
  }
  // The native-look kit (a-btn, a-chip, a-row, ...), after the tokens and before the piece's own styles
  function kit(css) {
    var theme = document.getElementById('acpira-theme');
    var el = document.createElement('style');
    el.id = 'acpira-kit';
    el.textContent = css || '';
    document.head.insertBefore(el, theme ? theme.nextSibling : document.head.firstChild);
  }
  function tell(list, value) {
    list.slice().forEach(function (fn) { try { fn(value); } catch (err) { console.error(err); } });
  }
  function onMessage(e) {
    if (e.source !== parent) return;
    var m = e.data || {};
    if (m.acpira === 'load') {
      window.acpira.options = m.options || {};
      if (m.state) window.acpira.state = m.state;
      if (m.session) window.acpira.session = m.session;
      document.open();
      document.write(m.html || '');
      document.close();
      style(m.css);
      kit(m.kit);
      // document.open drops the window's listeners; the acpira object stays
      window.addEventListener('message', onMessage);
    } else if (m.acpira === 'theme') {
      style(m.css);
    } else if (m.acpira === 'result') {
      var w = waits[m.id];
      if (!w) return;
      delete waits[m.id];
      if (m.error !== undefined) w.reject(new Error(m.error)); else w.resolve(m.result);
    } else if (m.acpira === 'post') {
      tell(hearers, m.message);
    } else if (m.acpira === 'state') {
      window.acpira.state = m.state || {};
      tell(watchers, window.acpira.state);
    } else if (m.acpira === 'session') {
      window.acpira.session = m.session || null;
      tell(readers, window.acpira.session);
    }
  }
  window.acpira = {
    call: function (method, params) {
      return new Promise(function (resolve, reject) {
        var id = ++next;
        waits[id] = { resolve: resolve, reject: reject };
        parent.postMessage({ acpira: 'call', id: id, method: method, params: params }, '*');
      });
    },
    on: function (fn) {
      hearers.push(fn);
      return function () { hearers = hearers.filter(function (h) { return h !== fn; }); };
    },
    onState: function (fn) {
      watchers.push(fn);
      if (window.acpira.state) fn(window.acpira.state);
      return function () { watchers = watchers.filter(function (h) { return h !== fn; }); };
    },
    // The conversation's shape per exchange (no text); null with no session on screen
    onSession: function (fn) {
      readers.push(fn);
      if (window.acpira.session !== undefined) fn(window.acpira.session);
      return function () { readers = readers.filter(function (h) { return h !== fn; }); };
    },
    scrollTo: function (turn) {
      parent.postMessage({ acpira: 'scrollTo', turn: turn }, '*');
    },
    options: {}
  };
  window.addEventListener('message', onMessage);
  parent.postMessage({ acpira: 'ready' }, '*');
})();
</script>
</body>
</html>
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageHost {
  Editor,
  Browser,
}

impl PageHost {
  pub fn as_str(self) -> &'static str {
    match self {
      PageHost::Editor => "editor",
      PageHost::Browser => "browser",
    }
  }
}

#[derive(Debug, Clone)]
pub struct PageConfig {
  pub host: PageHost,
  pub locale: String,
  pub dark: bool,
  /// appearance.font, validated by the caller
  pub font: Option<String>,
  /// A browser tab of a chat window's tool panel (host browser in a frame): the page talks through its parent
  /// (webview/panel/browserTabs.ts), there being no shell transport in a frame
  pub tab: Option<String>,
  /// The data folder is missing and no engine runs: the page starts the first-run flow (first_run.rs)
  pub first_run: Option<FirstRunPage>,
}

/// What the first-run flow needs before any engine: the folder as the consent card names it, and the language to speak
#[derive(Debug, Clone)]
pub struct FirstRunPage {
  pub home: String,
  pub locale: String,
}

fn escape_attr(s: &str) -> String {
  s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

/// JSON safe inside a <script>
fn script_json(v: &serde_json::Value) -> String {
  v.to_string().replace('<', "\\u003c")
}

pub fn page_html(c: &PageConfig) -> String {
  page_html_on(c, APP_ORIGIN, PLATFORM)
}

/// page_html for a given origin and platform (tests cover both shapes on one machine)
pub fn page_html_on(c: &PageConfig, origin: &str, platform: &str) -> String {
  let nonce = uuid::Uuid::new_v4().simple().to_string();
  let src = csp_source(origin);
  let src = src.as_str();
  let mut csp = vec![
    "default-src 'none'".to_owned(),
    format!("img-src {src} https: data:"),
    // Attachment text blobs are fetched for the peek card; loopback pages are probed before open_browser shows them;
    // ipc: carries the shell transport's messages (Tauri's IPC, with http://ipc.localhost as its fallback)
    format!("connect-src {src} ipc: http://ipc.localhost http://localhost:* http://127.0.0.1:* https://localhost:* https://127.0.0.1:*"),
    format!("style-src {src} 'unsafe-inline'"),
    format!("font-src {src} data:"),
    format!("script-src 'nonce-{nonce}' 'wasm-unsafe-eval'"),
    "worker-src blob:".to_owned(),
    // The attention chime is a WAV synthesized in the page (effects/chime.ts); the tool panel plays local files from /file
    format!("media-src blob: {src}"),
  ];
  match c.host {
    // The browser panel frames the preview proxy, which listens on loopback (the engine runs on this machine)
    PageHost::Browser => csp.push("frame-src http://127.0.0.1:* http://localhost:*".to_owned()),
    // The tool panel's browser tabs are this app's own browser page, framed
    PageHost::Editor => csp.push(format!("frame-src {src}")),
  }
  let mut config = json!({
    "host": c.host.as_str(),
    "sessionLinkBase": SESSION_LINK_BASE,
    "shell": "desktop",
    "platform": platform,
    "fileBase": format!("{origin}/file"),
  });
  if let Some(f) = &c.font {
    config["font"] = json!(f);
  }
  if let Some(f) = &c.first_run {
    config["firstRun"] = json!({ "home": f.home, "locale": f.locale });
  }
  let mut boot = format!("window.__acpira={}", script_json(&config));
  if let Some(tab) = &c.tab {
    config["tab"] = json!(tab);
    boot = format!("window.__acpira={};window.__acpiraApi=parent.__acpiraBrowserTab({})", script_json(&config), script_json(&json!(tab)));
  }
  format!(
    r#"<!doctype html>
<html lang="{lang}">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="{csp}">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Acpira</title>
<link rel="stylesheet" href="{origin}/webview/main.css">
<link rel="stylesheet" href="{origin}/desktop.css">
</head>
<body class="{scheme}">
<div id="root"></div>
<script nonce="{nonce}">{boot}</script>
<script type="module" nonce="{nonce}" src="{origin}/webview/main.js"></script>
</body>
</html>"#,
    lang = escape_attr(&c.locale),
    csp = csp.join("; "),
    scheme = if c.dark { "vscode-dark" } else { "vscode-light" },
  )
}

/// What a request to the app protocol resolves to
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
  /// The page itself, for this host, and the tool panel tab it is (an id is all a tab may carry into the page)
  Page {
    host: PageHost,
    tab: Option<String>,
  },
  Theme,
  PluginFrame,
  File(PathBuf),
  /// A local media file by absolute path (`/file?path=`), served in byte ranges so a long video seeks without being read whole
  Media {
    file: PathBuf,
    mime: &'static str,
  },
  /// A bundle file the packaged app stores zstd-compressed beside its name (`<name>.zst`, apps/desktop/scripts/package.mjs
  /// packs `assets/`); `mime` is the original name's. `unpack` reads it
  Packed {
    file: PathBuf,
    mime: &'static str,
  },
  NotFound,
}

/// The original bytes of a `Route::Packed` file
pub fn unpack(file: &Path) -> std::io::Result<Vec<u8>> {
  zstd::stream::decode_all(std::fs::File::open(file)?)
}

/// `path` and `query` of an acpira-app URL; `webview` holds the bundle, `blobs` the engine's sessions directory (where
/// attachment blobs live) once the engine said where it is
pub fn route(path: &str, query: Option<&str>, webview: &Path, blobs: Option<&Path>) -> Route {
  let param = |name: &str| {
    query
      .unwrap_or("")
      .split('&')
      .find_map(|kv| kv.strip_prefix(name).and_then(|r| r.strip_prefix('=')))
      .and_then(crate::confine::percent_decode)
  };
  if path == "/file" {
    // Any absolute path, but only types a page shows inert: HTML or SVG would run their scripts with the app's origin
    let file = param("path").map(PathBuf::from).filter(|f| f.is_absolute() && f.is_file());
    return file.and_then(|file| media_mime(&file).map(|mime| Route::Media { file, mime })).unwrap_or(Route::NotFound);
  }
  if path == "/index.html" || path == "/" {
    let host = if param("host").as_deref() == Some("browser") { PageHost::Browser } else { PageHost::Editor };
    let tab = (host == PageHost::Browser)
      .then(|| param("tab"))
      .flatten()
      .filter(|t| (1..=64).contains(&t.len()) && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
    return Route::Page { host, tab };
  }
  if path == "/desktop.css" {
    return Route::Theme;
  }
  if path == "/plugin-frame.html" {
    return Route::PluginFrame;
  }
  let file = if let Some(rest) = path.strip_prefix("/webview/") {
    let plain = crate::confine::confine(webview, rest).filter(|f| f.is_file());
    if plain.is_none()
      && let Some(file) = crate::confine::confine(webview, &format!("{rest}.zst")).filter(|f| f.is_file())
    {
      let mime = mime(&file.with_extension(""));
      return Route::Packed { file, mime };
    }
    plain
  } else if let (Some(rest), Some(root)) = (path.strip_prefix("/blobs/"), blobs) {
    crate::confine::confine(root, rest)
  } else {
    None
  };
  file.filter(|f| f.is_file()).map_or(Route::NotFound, Route::File)
}

/// The type of a file `/file` serves: images, audio, video and PDF, never SVG (a script-bearing document)
pub fn media_mime(path: &Path) -> Option<&'static str> {
  let m = mime(path);
  let media = (m.starts_with("image/") && m != "image/svg+xml") || m.starts_with("audio/") || m.starts_with("video/") || m == "application/pdf";
  media.then_some(m)
}

/// What a `Range` header asks of a file
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteRange {
  /// No usable range: the whole file answers
  Whole,
  /// Inclusive offsets (206)
  Span(u64, u64),
  /// Past the end (416)
  Unsatisfiable,
}

/// The first range of a `Range: bytes=…` header for a file of `len` bytes, at most `max` long (a player asking `bytes=0-`
/// of a long video gets the head and asks on)
pub fn byte_range(header: Option<&str>, len: u64, max: u64) -> ByteRange {
  let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else { return ByteRange::Whole };
  let first = spec.split(',').next().unwrap_or("").trim();
  let Some((a, b)) = first.split_once('-') else { return ByteRange::Whole };
  let (start, end) = match (a.trim().parse::<u64>().ok(), b.trim().parse::<u64>().ok()) {
    (Some(s), Some(e)) if s <= e => (s, e.min(len.saturating_sub(1))),
    (Some(s), None) if b.trim().is_empty() => (s, len.saturating_sub(1)),
    // A suffix: the last n bytes
    (None, Some(n)) if a.trim().is_empty() && n > 0 => (len.saturating_sub(n), len.saturating_sub(1)),
    _ => return ByteRange::Whole,
  };
  if len == 0 || start >= len {
    return ByteRange::Unsatisfiable;
  }
  ByteRange::Span(start, end.min(start.saturating_add(max.max(1) - 1)))
}

pub fn mime(path: &Path) -> &'static str {
  match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
    "html" => "text/html; charset=utf-8",
    "js" | "mjs" => "text/javascript; charset=utf-8",
    "css" => "text/css; charset=utf-8",
    "json" | "map" => "application/json",
    "txt" | "md" | "log" => "text/plain; charset=utf-8",
    "svg" => "image/svg+xml",
    "png" => "image/png",
    "jpg" | "jpeg" => "image/jpeg",
    "gif" => "image/gif",
    "webp" => "image/webp",
    "avif" => "image/avif",
    "ico" => "image/x-icon",
    "bmp" => "image/bmp",
    "tif" | "tiff" => "image/tiff",
    "heic" => "image/heic",
    "heif" => "image/heif",
    "woff2" => "font/woff2",
    "woff" => "font/woff",
    "ttf" => "font/ttf",
    "otf" => "font/otf",
    "wasm" => "application/wasm",
    "wav" => "audio/wav",
    "mp3" => "audio/mpeg",
    "m4a" => "audio/mp4",
    "aac" => "audio/aac",
    "flac" => "audio/flac",
    "ogg" | "oga" | "opus" => "audio/ogg",
    "mp4" | "m4v" => "video/mp4",
    "mov" => "video/quicktime",
    "webm" => "video/webm",
    "ogv" => "video/ogg",
    "pdf" => "application/pdf",
    _ => "application/octet-stream",
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_page_carries_the_config_a_nonce_and_a_csp_for_its_host() {
    let html = page_html(&PageConfig { host: PageHost::Editor, locale: "zh-CN".into(), dark: true, font: Some("inter".into()), tab: None, first_run: None });
    assert!(html.contains(r#"<html lang="zh-CN">"#));
    assert!(html.contains(r#"<body class="vscode-dark">"#));
    assert!(html.contains(&format!(
      r#"window.__acpira={{"host":"editor","sessionLinkBase":"acpira://session/","shell":"desktop","platform":"{PLATFORM}","fileBase":"{APP_ORIGIN}/file","font":"inter"}}"#
    )));
    let mac = page_html_on(&PageConfig { host: PageHost::Editor, locale: "en".into(), dark: true, font: None, tab: None, first_run: None }, app_origin(false), "mac");
    assert!(mac.contains("frame-src acpira-app:") && mac.contains("connect-src acpira-app: ipc: http://ipc.localhost"));
    assert!(mac.contains(r#"src="acpira-app://localhost/webview/main.js""#));
    // WebView2: the bundle comes from the http origin wry maps the scheme to
    let win = page_html_on(&PageConfig { host: PageHost::Editor, locale: "en".into(), dark: true, font: None, tab: None, first_run: None }, app_origin(true), "win");
    assert!(win.contains("frame-src http://acpira-app.localhost\"") && win.contains("img-src http://acpira-app.localhost https: data:"));
    assert!(win.contains(r#"href="http://acpira-app.localhost/webview/main.css""#) && win.contains(r#""platform":"win""#));
    let nonce = html.split("'nonce-").nth(1).unwrap().split('\'').next().unwrap();
    assert_eq!(html.matches(&format!(r#"nonce="{nonce}""#)).count(), 2);
    let tab = page_html(&PageConfig { host: PageHost::Browser, locale: "en".into(), dark: false, font: None, tab: Some("t-1".into()), first_run: None });
    assert!(tab.contains(r#"window.__acpiraApi=parent.__acpiraBrowserTab("t-1")"#));
    assert!(tab.contains("frame-src http://127.0.0.1:* http://localhost:*") && tab.contains("vscode-light"));
    // A font name cannot close the script
    let evil =
      page_html(&PageConfig { host: PageHost::Editor, locale: "en\"><x".into(), dark: false, font: Some("</script>".into()), tab: None, first_run: None });
    assert!(!evil.contains("</script><") && evil.contains(r#"lang="en&quot;>&lt;x""#));
    let first = page_html(&PageConfig {
      host: PageHost::Editor,
      locale: "en".into(),
      dark: true,
      font: None,
      tab: None,
      first_run: Some(FirstRunPage { home: "~/.acpira</script>".into(), locale: "zh-CN".into() }),
    });
    assert!(first.contains(r#""firstRun":{"home":"~/.acpira\u003c/script>","locale":"zh-CN"}"#));
  }

  #[test]
  fn routes_the_page_the_theme_and_confined_files() {
    let d = tempfile::tempdir().unwrap();
    let web = d.path().join("webview");
    let blobs = d.path().join("sessions");
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::create_dir_all(&blobs).unwrap();
    std::fs::write(web.join("main.js"), "").unwrap();
    std::fs::write(blobs.join("b.png"), "").unwrap();
    std::fs::write(d.path().join("secret"), "").unwrap();
    assert_eq!(route("/index.html", Some("host=editor"), &web, None), Route::Page { host: PageHost::Editor, tab: None });
    assert_eq!(
      route("/index.html", Some("host=browser&tab=a_1"), &web, None),
      Route::Page { host: PageHost::Browser, tab: Some("a_1".into()) }
    );
    assert_eq!(route("/index.html", Some("host=browser&tab=%3Cx%3E"), &web, None), Route::Page { host: PageHost::Browser, tab: None });
    assert_eq!(route("/desktop.css", None, &web, None), Route::Theme);
    assert_eq!(route("/plugin-frame.html", None, &web, None), Route::PluginFrame);
    assert!(matches!(route("/webview/main.js", None, &web, None), Route::File(f) if f.ends_with("main.js")));
    assert_eq!(route("/webview/../secret", None, &web, None), Route::NotFound);
    assert_eq!(route("/webview/assets", None, &web, None), Route::NotFound);
    assert_eq!(route("/blobs/b.png", None, &web, None), Route::NotFound);
    assert!(matches!(route("/blobs/b.png", None, &web, Some(&blobs)), Route::File(_)));
    assert_eq!(route("/other", None, &web, Some(&blobs)), Route::NotFound);
    // A packed bundle file answers under its own name, with its own type; a plain file of that name wins
    std::fs::write(web.join("assets/c-AbCd1234.js.zst"), zstd::encode_all(&b"export {}"[..], 3).unwrap()).unwrap();
    let packed = route("/webview/assets/c-AbCd1234.js", None, &web, None);
    let Route::Packed { file, mime: m } = &packed else { panic!("{packed:?}") };
    assert_eq!((unpack(file).unwrap().as_slice(), *m), (&b"export {}"[..], "text/javascript; charset=utf-8"));
    std::fs::write(web.join("assets/c-AbCd1234.js"), "").unwrap();
    assert!(matches!(route("/webview/assets/c-AbCd1234.js", None, &web, None), Route::File(_)));
    // No packed fallback outside the bundle, nor past its root
    std::fs::write(blobs.join("x.png.zst"), "").unwrap();
    assert_eq!(route("/blobs/x.png", None, &web, Some(&blobs)), Route::NotFound);
    std::fs::write(d.path().join("secret2.zst"), "").unwrap();
    assert_eq!(route("/webview/../secret2", None, &web, None), Route::NotFound);
    assert_eq!(mime(Path::new("a/B.PNG")), "image/png");
  }

  #[test]
  fn serves_local_media_by_absolute_path_and_nothing_else() {
    let d = tempfile::tempdir().unwrap();
    let web = d.path().join("webview");
    for name in ["clip.MP4", "doc.pdf", "a b.png", "x.svg", "page.html", "notes.txt"] {
      std::fs::write(d.path().join(name), "").unwrap();
    }
    let ask = |name: &str| {
      let q = format!("path={}", d.path().join(name).to_string_lossy().replace('%', "%25").replace(' ', "%20"));
      route("/file", Some(&q), &web, None)
    };
    assert!(matches!(ask("clip.MP4"), Route::Media { mime: "video/mp4", .. }));
    assert!(matches!(ask("doc.pdf"), Route::Media { mime: "application/pdf", .. }));
    assert!(matches!(ask("a b.png"), Route::Media { mime: "image/png", .. }));
    for refused in ["x.svg", "page.html", "notes.txt", "gone.mp4"] {
      assert_eq!(ask(refused), Route::NotFound, "{refused}");
    }
    assert_eq!(route("/file", Some("path=clip.MP4"), &web, None), Route::NotFound);
    assert_eq!(route("/file", None, &web, None), Route::NotFound);
  }

  #[test]
  fn byte_ranges_follow_the_header_capped() {
    use ByteRange::{Span, Unsatisfiable, Whole};
    assert_eq!(byte_range(None, 100, 10), Whole);
    assert_eq!(byte_range(Some("bytes=0-1"), 100, 10), Span(0, 1));
    assert_eq!(byte_range(Some("bytes=0-"), 100, 10), Span(0, 9));
    assert_eq!(byte_range(Some("bytes=95-200"), 100, 10), Span(95, 99));
    assert_eq!(byte_range(Some("bytes=-5"), 100, 10), Span(95, 99));
    assert_eq!(byte_range(Some("bytes=10-20, 30-40"), 100, 100), Span(10, 20));
    assert_eq!(byte_range(Some("bytes=100-"), 100, 10), Unsatisfiable);
    assert_eq!(byte_range(Some("bytes=0-"), 0, 10), Unsatisfiable);
    assert_eq!(byte_range(Some("bytes=5-2"), 100, 10), Whole);
    assert_eq!(byte_range(Some("items=0-1"), 100, 10), Whole);
  }

  #[test]
  fn only_the_app_origin_counts_as_the_app() {
    for windows in [false, true] {
      let o = app_origin(windows);
      assert!(is_app_url(&format!("{o}/index.html?host=editor"), o));
      assert!(is_app_url(&format!("{o}/blank"), o));
    }
    assert!(!is_app_url("http://acpira-app.localhost.evil.com/", app_origin(true)));
    assert!(!is_app_url("http://acpira-app.localhost:8080/", app_origin(true)));
    assert!(!is_app_url("https://acpira-app.localhost/", app_origin(true)));
    assert!(!is_app_url("acpira-app://elsewhere/", app_origin(false)));
    assert!(!is_app_url("http://localhost/", app_origin(true)));
  }
}
