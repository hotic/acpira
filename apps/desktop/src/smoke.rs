//! The smoke run's hook (scripts/smoke.mjs), only in a build with the `smoke` feature: a Unix socket (a named pipe on
//! Windows) at ACPIRA_DESKTOP_HOOK takes one JSON command per line and answers each with `{ id, ok, value }`. Commands: `eval` (a
//! page expression, awaited, its JSON back), `windows`, `resize`, `wheel`, `close`, `menuBar`, `menu`, `link`, `screenshot`, `update`, `quit`. WKWebView has no
//! DevTools protocol, so this is how a script drives the real app

use std::collections::HashMap;

use parking_lot::Mutex;
#[cfg(feature = "smoke")]
use serde_json::{Value, json};
use tokio::sync::oneshot;

/// A smoke build starts the updated bundle's executable itself, so the new process keeps the run's temp home; a
/// release build goes through `open` (LaunchServices)
pub const RELAUNCH_DIRECT: bool = cfg!(feature = "smoke");

/// The smoke run's updater: its local release server (`ACPIRA_DESKTOP_UPDATE_URL`, plain http on the loopback) signed with
/// a throwaway key (`ACPIRA_DESKTOP_UPDATE_KEY`, hex), and the ad hoc package's signature checked by identifier only.
/// The run's copy lives under the temp folder, so the Windows exe's temp-folder check (`install_blocker`) is left out.
/// None in a release build, or when the run does not test updates (so a smoke never asks GitHub)
#[cfg(feature = "smoke")]
pub fn update_config(
  installed: Option<&std::path::Path>,
) -> Option<(acpira_desktop::update::Config, Option<acpira_desktop::update::ManualReason>)> {
  use acpira_desktop::update::{self, ManualReason};
  let url = std::env::var("ACPIRA_DESKTOP_UPDATE_URL").ok().filter(|u| !u.is_empty());
  let keys = url.as_ref().and_then(|_| std::env::var("ACPIRA_DESKTOP_UPDATE_KEY").ok()).and_then(|k| update::manifest::key_of_hex(&k));
  let config = update::Config {
    manifest_url: url.unwrap_or_default(),
    keys: keys.into_iter().collect(),
    policy: update::fetch::Policy { hosts: vec![], loopback_http: true },
    // An ad hoc bundle's identifier on macOS; a Windows exe is checked by its file list only (stage.rs)
    requirement: cfg!(target_os = "macos").then(|| format!("identifier \"{}\"", update::stage::IDENTIFIER)),
    notarized: false,
    release_url: update::RELEASE_URL.into(),
  };
  let manual = match installed {
    None => Some(ManualReason::NotBundled),
    Some(app) => update::swap::location_blocker(app),
  };
  Some((config, manual))
}

#[cfg(not(feature = "smoke"))]
pub fn update_config(
  _installed: Option<&std::path::Path>,
) -> Option<(acpira_desktop::update::Config, Option<acpira_desktop::update::ManualReason>)> {
  None
}

/// The page's answer to one `eval`: whether it ran, and its JSON (or the error)
type Answer = oneshot::Sender<(bool, String)>;

static PENDING: Mutex<Option<HashMap<u64, Answer>>> = Mutex::new(None);

/// The page's answer to an `eval`
#[tauri::command]
pub fn acpira_smoke_result(id: u64, ok: bool, value: String) {
  if let Some(tx) = PENDING.lock().as_mut().and_then(|p| p.remove(&id)) {
    let _ = tx.send((ok, value));
  }
}

#[cfg(feature = "smoke")]
pub fn start() {
  let Some(path) = std::env::var_os("ACPIRA_DESKTOP_HOOK") else { return };
  tauri::async_runtime::spawn(listen(std::path::PathBuf::from(path)));
}

/// A Unix socket at the hook's path
#[cfg(all(feature = "smoke", unix))]
async fn listen(path: std::path::PathBuf) {
  let _ = std::fs::remove_file(&path);
  let listener = match tokio::net::UnixListener::bind(&path) {
    Ok(l) => l,
    Err(e) => {
      crate::app::log(&format!("smoke hook: {e}"));
      return;
    }
  };
  crate::app::log(&format!("smoke hook listening on {}", path.display()));
  while let Ok((stream, _)) = listener.accept().await {
    tauri::async_runtime::spawn(serve_hook(stream));
  }
}

/// A named pipe (`\\.\pipe\…`) on Windows, local clients only; a new instance waits while one serves
#[cfg(all(feature = "smoke", windows))]
async fn listen(path: std::path::PathBuf) {
  use tokio::net::windows::named_pipe::ServerOptions;
  let create = |first: bool| ServerOptions::new().first_pipe_instance(first).reject_remote_clients(true).create(&path);
  let mut server = match create(true) {
    Ok(s) => s,
    Err(e) => {
      crate::app::log(&format!("smoke hook: {e}"));
      return;
    }
  };
  crate::app::log(&format!("smoke hook listening on {}", path.display()));
  loop {
    if server.connect().await.is_err() {
      return;
    }
    let next = match create(false) {
      Ok(s) => s,
      Err(e) => {
        crate::app::log(&format!("smoke hook: {e}"));
        return;
      }
    };
    tauri::async_runtime::spawn(serve_hook(std::mem::replace(&mut server, next)));
  }
}

/// One connection: a JSON command per line, each answered with `{ id, ok, value }`
#[cfg(feature = "smoke")]
async fn serve_hook(stream: impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static) {
  use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
  let (read, mut write) = tokio::io::split(stream);
  let mut lines = BufReader::new(read).lines();
  while let Ok(Some(line)) = lines.next_line().await {
    let cmd: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
    let id = cmd["id"].clone();
    let (ok, value) = run(cmd).await;
    let mut out = json!({ "id": id, "ok": ok, "value": value }).to_string();
    out.push('\n');
    if write.write_all(out.as_bytes()).await.is_err() {
      break;
    }
  }
}

#[cfg(not(feature = "smoke"))]
pub fn start() {}

#[cfg(feature = "smoke")]
fn window(cmd: &Value) -> Option<std::sync::Arc<crate::chat_window::ChatWindow>> {
  let a = crate::app::app();
  match cmd["window"].as_str() {
    Some(label) => a.window_by_label(label),
    None => a.focused_window(),
  }
}

#[cfg(feature = "smoke")]
async fn on_main<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
  let (tx, rx) = oneshot::channel();
  crate::app::app()
    .handle
    .run_on_main_thread(move || {
      let _ = tx.send(f());
    })
    .ok()?;
  rx.await.ok()
}

#[cfg(feature = "smoke")]
async fn run(cmd: Value) -> (bool, Value) {
  use std::sync::atomic::{AtomicU64, Ordering};
  static SEQ: AtomicU64 = AtomicU64::new(0);
  let a = crate::app::app();
  match cmd["op"].as_str().unwrap_or_default() {
    "eval" => {
      let Some(w) = window(&cmd) else { return (false, json!("no such window")) };
      let id = SEQ.fetch_add(1, Ordering::SeqCst) + 1;
      let (tx, rx) = oneshot::channel();
      PENDING.lock().get_or_insert_with(HashMap::new).insert(id, tx);
      let js = cmd["js"].as_str().unwrap_or("undefined");
      // A failure answers with the error and its stack: WebKit's stack leaves the message out, V8's starts with it
      let script = format!(
        "(async () => {{ let ok = true, value; try {{ value = JSON.stringify(await ({js}) ?? null); }} catch (e) {{ ok = false; value = String(e); if (e && e.stack && !String(e.stack).includes(value)) value += '\\n' + e.stack; }} window.__TAURI_INTERNALS__.invoke('acpira_smoke_result', {{ id: {id}, ok, value }}); }})()"
      );
      if w.window.eval(script).is_err() {
        return (false, json!("eval failed"));
      }
      match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
        Ok(Ok((ok, value))) => (ok, if ok { serde_json::from_str(&value).unwrap_or(Value::Null) } else { json!(value) }),
        _ => (false, json!("the page did not answer")),
      }
    }
    "windows" => {
      let list = on_main(|| {
        crate::app::app()
          .windows()
          .iter()
          .map(|w| {
            let scale = w.window.scale_factor().unwrap_or(1.0);
            let size = w.window.inner_size().map(|s| s.to_logical::<f64>(scale)).ok();
            json!({ "label": w.label, "session": w.session_id(), "ready": w.ready(), "visible": w.window.is_visible().unwrap_or(false),
              "focused": w.window.is_focused().unwrap_or(false), "width": size.map(|s| s.width), "height": size.map(|s| s.height) })
          })
          .collect::<Vec<_>>()
      })
      .await;
      (true, json!(list.unwrap_or_default()))
    }
    "attention" => (true, json!(a.attention.lock().clone())),
    "resize" => {
      let Some(w) = window(&cmd) else { return (false, json!("no such window")) };
      let (wd, ht) = (cmd["width"].as_f64().unwrap_or(1180.0), cmd["height"].as_f64().unwrap_or(820.0));
      let ok = w.window.set_size(tauri::LogicalSize::new(wd, ht)).is_ok();
      (ok, Value::Null)
    }
    "wheel" => {
      // Classic wheel notches at a point of the window's content, through the app's own event queue (mac.rs post_wheel_notch)
      let Some(w) = window(&cmd) else { return (false, json!("no such window")) };
      let (x, y) = (cmd["x"].as_f64().unwrap_or(400.0), cmd["y"].as_f64().unwrap_or(400.0));
      let lines = cmd["lines"].as_i64().unwrap_or(-3) as i32;
      #[cfg(target_os = "macos")]
      {
        let ok = on_main(move || crate::mac::post_wheel_notch(&w.window, x, y, lines)).await.unwrap_or(false);
        (ok, Value::Null)
      }
      #[cfg(not(target_os = "macos"))]
      {
        let _ = (w, x, y, lines);
        (false, json!("macOS only"))
      }
    }
    "close" => {
      // The window's close button: the same CloseRequested path a click takes
      let Some(w) = window(&cmd) else { return (false, json!("no such window")) };
      let ok = on_main(move || w.window.close().is_ok()).await.unwrap_or(false);
      (ok, Value::Null)
    }
    "menuBar" => {
      // The installed menu bar's top-level titles, as macOS shows them; elsewhere the menus the page draws (desktopMenu)
      let titles = if cfg!(target_os = "macos") {
        on_main(|| {
          let menu = crate::app::app().handle.menu()?;
          Some(menu.items().ok()?.iter().filter_map(|i| i.as_submenu().and_then(|s| s.text().ok())).collect::<Vec<_>>())
        })
        .await
        .flatten()
      } else {
        crate::app::app().page_menu_titles()
      };
      (titles.is_some(), json!(titles.unwrap_or_default()))
    }
    "menu" => {
      let id = cmd["item"].as_str().unwrap_or_default().to_owned();
      on_main(move || crate::menu::on_event(&crate::app::app().handle, &id)).await;
      (true, Value::Null)
    }
    "link" => {
      let url = cmd["url"].as_str().unwrap_or_default().to_owned();
      on_main(move || crate::app::app().take_link(&url)).await;
      (true, Value::Null)
    }
    "platform" => {
      // A platform request as the engine sends one (reveal, open, a terminal, …), run where the engine's run
      let request = match serde_json::from_value::<acpira_shared::sidecar::PlatformRequest>(cmd["request"].clone()) {
        Ok(r) => r,
        Err(e) => return (false, json!(e.to_string())),
      };
      let answer = on_main(move || acpira_desktop::platform::handle_request(request, &**crate::app::app())).await;
      match answer {
        Some(Ok(v)) => (true, v),
        Some(Err(e)) => (false, json!(e)),
        None => (false, json!("the app is quitting")),
      }
    }
    "notify" => {
      // A system notification as the app would show one (none in a background run)
      let (title, body) = (cmd["title"].as_str().unwrap_or("Acpira").to_owned(), cmd["body"].as_str().unwrap_or_default().to_owned());
      let session = cmd["session"].as_str().map(str::to_owned);
      on_main(move || crate::app::app().system_notification(&title, &body, session.as_deref())).await;
      (true, Value::Null)
    }
    "screenshot" => {
      let Some(w) = window(&cmd) else { return (false, json!("no such window")) };
      let path = cmd["path"].as_str().unwrap_or("/tmp/acpira-shot.png").to_owned();
      #[cfg(target_os = "macos")]
      {
        match crate::mac::screenshot(&w.window, &path).await {
          Ok(()) => (true, json!(path)),
          Err(e) => (false, json!(e)),
        }
      }
      #[cfg(target_os = "linux")]
      {
        let saved = on_main(move || crate::linux::screenshot(&w.window, &path).map(|()| path)).await;
        match saved {
          Some(Ok(path)) => (true, json!(path)),
          Some(Err(e)) => (false, json!(e)),
          None => (false, json!("the app is quitting")),
        }
      }
      #[cfg(windows)]
      {
        let _ = w;
        (false, json!(format!("no screenshots on this platform yet: {path}")))
      }
    }
    "update" => {
      // The updater's state as the page sees it
      let state = a.updater().map(|u| u.state());
      (state.is_some(), serde_json::to_value(state).unwrap_or(Value::Null))
    }
    "quit" => {
      on_main(|| {
        let a = crate::app::app();
        a.before_quit();
        a.handle.exit(0);
      })
      .await;
      (true, Value::Null)
    }
    other => {
      let _ = a;
      (false, json!(format!("unknown op {other}")))
    }
  }
}
