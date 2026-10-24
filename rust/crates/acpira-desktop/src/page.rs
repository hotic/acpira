//! What the app protocol serves (acpira-app://localhost): the HTML around the webview bundle, the theme sheet, the
//! bundle's files and attachment blobs. Mirrors the VS Code shell's page (apps/vscode/src/bridge.ts html()): the same CSP
//! shape with the app protocol as the resource source, the same inline config the bundle reads before init
//! (window.__acpira), plus `shell: 'desktop'` for the desktop-only branches

use std::path::{Path, PathBuf};

use serde_json::json;

/// WKWebView serves a custom scheme as `<scheme>://localhost` in Tauri's convention; a standard, secure origin
pub const APP_ORIGIN: &str = "acpira-app://localhost";
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
}

fn escape_attr(s: &str) -> String {
  s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

/// JSON safe inside a <script>
fn script_json(v: &serde_json::Value) -> String {
  v.to_string().replace('<', "\\u003c")
}

pub fn page_html(c: &PageConfig) -> String {
  let nonce = uuid::Uuid::new_v4().simple().to_string();
  let src = "acpira-app:";
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
    // The attention chime is a WAV synthesized in the page (effects/chime.ts)
    "media-src blob:".to_owned(),
  ];
  match c.host {
    // The browser panel frames the preview proxy, which listens on loopback (the engine runs on this machine)
    PageHost::Browser => csp.push("frame-src http://127.0.0.1:* http://localhost:*".to_owned()),
    // The tool panel's browser tabs are this app's own browser page, framed
    PageHost::Editor => csp.push(format!("frame-src {src}")),
  }
  let mut config = json!({ "host": c.host.as_str(), "sessionLinkBase": SESSION_LINK_BASE, "shell": "desktop" });
  if let Some(f) = &c.font {
    config["font"] = json!(f);
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
<link rel="stylesheet" href="{APP_ORIGIN}/webview/main.css">
<link rel="stylesheet" href="{APP_ORIGIN}/desktop.css">
</head>
<body class="{scheme}">
<div id="root"></div>
<script nonce="{nonce}">{boot}</script>
<script type="module" nonce="{nonce}" src="{APP_ORIGIN}/webview/main.js"></script>
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
  File(PathBuf),
  NotFound,
}

/// `path` and `query` of an acpira-app URL; `webview` holds the bundle, `blobs` the engine's sessions directory (where
/// attachment blobs live) once the engine said where it is
pub fn route(path: &str, query: Option<&str>, webview: &Path, blobs: Option<&Path>) -> Route {
  if path == "/index.html" || path == "/" {
    let param = |name: &str| {
      query
        .unwrap_or("")
        .split('&')
        .find_map(|kv| kv.strip_prefix(name).and_then(|r| r.strip_prefix('=')))
        .and_then(crate::confine::percent_decode)
    };
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
  let file = if let Some(rest) = path.strip_prefix("/webview/") {
    crate::confine::confine(webview, rest)
  } else if let (Some(rest), Some(root)) = (path.strip_prefix("/blobs/"), blobs) {
    crate::confine::confine(root, rest)
  } else {
    None
  };
  file.filter(|f| f.is_file()).map_or(Route::NotFound, Route::File)
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
    "woff2" => "font/woff2",
    "woff" => "font/woff",
    "ttf" => "font/ttf",
    "otf" => "font/otf",
    "wasm" => "application/wasm",
    "wav" => "audio/wav",
    "mp3" => "audio/mpeg",
    "mp4" => "video/mp4",
    "pdf" => "application/pdf",
    _ => "application/octet-stream",
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_page_carries_the_config_a_nonce_and_a_csp_for_its_host() {
    let html = page_html(&PageConfig { host: PageHost::Editor, locale: "zh-CN".into(), dark: true, font: Some("inter".into()), tab: None });
    assert!(html.contains(r#"<html lang="zh-CN">"#));
    assert!(html.contains(r#"<body class="vscode-dark">"#));
    assert!(html.contains(r#"window.__acpira={"host":"editor","sessionLinkBase":"acpira://session/","shell":"desktop","font":"inter"}"#));
    assert!(html.contains("frame-src acpira-app:") && html.contains("connect-src acpira-app: ipc: http://ipc.localhost"));
    let nonce = html.split("'nonce-").nth(1).unwrap().split('\'').next().unwrap();
    assert_eq!(html.matches(&format!(r#"nonce="{nonce}""#)).count(), 2);
    let tab = page_html(&PageConfig { host: PageHost::Browser, locale: "en".into(), dark: false, font: None, tab: Some("t-1".into()) });
    assert!(tab.contains(r#"window.__acpiraApi=parent.__acpiraBrowserTab("t-1")"#));
    assert!(tab.contains("frame-src http://127.0.0.1:* http://localhost:*") && tab.contains("vscode-light"));
    // A font name cannot close the script
    let evil =
      page_html(&PageConfig { host: PageHost::Editor, locale: "en\"><x".into(), dark: false, font: Some("</script>".into()), tab: None });
    assert!(!evil.contains("</script><") && evil.contains(r#"lang="en&quot;>&lt;x""#));
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
    assert!(matches!(route("/webview/main.js", None, &web, None), Route::File(f) if f.ends_with("main.js")));
    assert_eq!(route("/webview/../secret", None, &web, None), Route::NotFound);
    assert_eq!(route("/webview/assets", None, &web, None), Route::NotFound);
    assert_eq!(route("/blobs/b.png", None, &web, None), Route::NotFound);
    assert!(matches!(route("/blobs/b.png", None, &web, Some(&blobs)), Route::File(_)));
    assert_eq!(route("/other", None, &web, Some(&blobs)), Route::NotFound);
    assert_eq!(mime(Path::new("a/B.PNG")), "image/png");
  }
}
