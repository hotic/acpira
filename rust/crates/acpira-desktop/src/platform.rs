//! IDE actions the engine asks the desktop shell for (PlatformRequest). Arguments arrive resolved and allowlisted; this
//! module decides what each one does, and the shell (`Platform`) carries it out with the system's own tools

use std::path::PathBuf;

use acpira_shared::sidecar::{BrowserOpened, PlanTarget, PlatformRequest, ToastOpen};
use serde_json::{Map, Value};

use crate::editors::{Editor, FileTarget};
use crate::terminal::TerminalRequest;

/// What hello declares; the engine falls back for the rest (searchFiles walks the project itself). openBrowser gives
/// agents the open_browser tool
pub const CAPABILITIES: [&str; 10] = [
  "openResolvedFile",
  "openPlanDocument",
  "revealInOS",
  "writeSetting",
  "openExternal",
  "openBrowser",
  "openInEditor",
  "runInTerminal",
  "toast",
  "notify",
];

/// The shell's side of the actions
pub trait Platform: Send + Sync {
  /// The shell's own store (writeSetting only carries its keys now); JSON null removes the key
  fn write_setting(&self, key: &str, value: Option<Value>);
  /// A setting as the shell sees it: shared keys from <ACPIRA_HOME>/settings.json
  fn setting(&self, key: &str) -> Option<Value>;
  fn log(&self, line: &str);
  /// Whether a window has focus and shows this session: the user is looking at it, so no chime or notification
  fn watching(&self, session_id: &str) -> bool;
  /// The page that plays the chime: the most recently focused one
  fn chime(&self);
  /// A system notification that brings the session up when clicked
  fn notification(&self, title: &str, body: &str, session_id: &str);
  /// A toast for the user: the toast stack of the page used last, a system notification while no window is open
  fn toast(&self, level: &str, text: &str, open: Option<ToastOpen>);
  /// "Open in new window" (the engine's openInEditor): a chat window of its own on that session
  fn open_window(&self, session_id: Option<String>);
  /// An agent's open_browser: a loopback page in the browser panel, any other http(s) page in the system browser
  fn open_browser(&self, url: &str, folder: Option<&str>) -> Result<BrowserOpened, String>;
  fn terminal(&self, r: TerminalRequest);
  /// "Open files with" an editor: false when its command line is not there
  fn open_in_editor(&self, editor: Editor, target: &FileTarget) -> bool;
  fn open_external(&self, url: &str) -> Result<(), String>;
  fn reveal(&self, path: &str) -> Result<(), String>;
  /// The system's default application for a file
  fn open_path(&self, path: &str) -> Result<(), String>;
  /// The app's language, for what the shell itself says
  fn t(&self, key: &str, params: &[(&str, &str)]) -> String;
}

pub fn handle_request(r: PlatformRequest, p: &dyn Platform) -> Result<Value, String> {
  match r {
    PlatformRequest::WriteSetting { key, value } => {
      p.write_setting(&key, Some(value));
      Ok(Value::Null)
    }
    PlatformRequest::OpenExternal { url } => p.open_external(&url).map(|_| Value::Null),
    PlatformRequest::RevealInOs { path } => p.reveal(&path).map(|_| Value::Null),
    PlatformRequest::OpenResolvedFile { path, line } => open_file(&path, line, p).map(|_| Value::Null),
    PlatformRequest::OpenPlanDocument { target } => {
      let path = match target {
        PlanTarget::Path { path } => path,
        PlanTarget::Markdown { markdown } => write_plan(&markdown).map_err(|e| e.to_string())?.to_string_lossy().into_owned(),
      };
      open_file(&path, None, p).map(|_| Value::Null)
    }
    PlatformRequest::Toast { level, text, open } => {
      p.toast(&level, &text, open);
      Ok(Value::Null)
    }
    PlatformRequest::OpenInEditor { session_id } => {
      p.open_window(session_id);
      Ok(Value::Null)
    }
    PlatformRequest::OpenBrowser { url, folder } => {
      p.open_browser(&url, folder.as_deref()).and_then(|o| serde_json::to_value(o).map_err(|e| e.to_string()))
    }
    PlatformRequest::RunInTerminal { title, command, args, env } => {
      p.terminal(TerminalRequest { title, command, args, env });
      Ok(Value::Null)
    }
    PlatformRequest::Notify { session_id, title, body, sound, system, .. } => {
      if p.watching(&session_id) {
        return Ok(Value::Null);
      }
      if sound {
        p.chime();
      }
      if system {
        p.notification(&title, &body, &session_id);
      }
      Ok(Value::Null)
    }
    other => Err(format!("{} is not supported by the desktop app", other.method())),
  }
}

/// A file link or a plan: in the editor the settings name, at the line, else (or when that editor is not installed) with
/// the system's default application
fn open_file(path: &str, line: Option<i64>, p: &dyn Platform) -> Result<(), String> {
  let opener = acpira_shared::settings::sanitize_setting("openFilesWith", &p.setting("openFilesWith").unwrap_or(Value::Null));
  if let Some(editor) = opener.as_str().and_then(Editor::parse) {
    if p.open_in_editor(editor, &FileTarget { path: path.to_owned(), line }) {
      return Ok(());
    }
    p.toast("info", &p.t("desktop.editorMissing", &[("editor", editor.name())]), None);
  }
  p.open_path(path)
}

/// A plan the agent only sent as markdown is written to a temp file first, so the default markdown app can open it
fn write_plan(markdown: &str) -> std::io::Result<PathBuf> {
  let dir = std::env::temp_dir().join(format!("acpira-plan-{}", uuid::Uuid::new_v4().simple()));
  std::fs::create_dir_all(&dir)?;
  let file = dir.join("plan.md");
  std::fs::write(&file, markdown)?;
  Ok(file)
}

/// The system's own openers: `open` on macOS, `xdg-open` on Linux, the shell API on Windows (ShellExecuteExW, and
/// SHOpenFolderAndSelectItems to reveal: `explorer` reports failure through an exit status it also gives on success)
pub mod system {
  #[cfg(not(windows))]
  fn run(program: &str, args: &[&str]) -> Result<(), String> {
    use std::process::{Command, Stdio};
    let out = Command::new(program)
      .args(args)
      .stdin(Stdio::null())
      .stdout(Stdio::null())
      .stderr(Stdio::piped())
      .output()
      .map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
      return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    Err(if err.is_empty() { format!("{program} failed ({})", out.status) } else { err })
  }

  pub fn open_external(url: &str) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:")) {
      return Err(format!("not an external link: {url}"));
    }
    open_any(url)
  }

  /// A file or a URL with the default application
  pub fn open_any(target: &str) -> Result<(), String> {
    #[cfg(windows)]
    return win::open(target);
    #[cfg(not(windows))]
    if cfg!(target_os = "macos") { run("open", &[target]) } else { run("xdg-open", &[target]) }
  }

  pub fn reveal(path: &str) -> Result<(), String> {
    #[cfg(windows)]
    return win::reveal(path);
    #[cfg(not(windows))]
    if cfg!(target_os = "macos") {
      run("open", &["-R", path])
    } else {
      run("xdg-open", &[std::path::Path::new(path).parent().and_then(|p| p.to_str()).unwrap_or(path)])
    }
  }

  #[cfg(windows)]
  mod win {
    use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
    use windows_sys::Win32::UI::Shell::{
      ILCreateFromPathW, ILFree, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, SHOpenFolderAndSelectItems, ShellExecuteExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(s: &str) -> Vec<u16> {
      s.encode_utf16().chain(Some(0)).collect()
    }

    /// The shell API wants COM on the calling thread; a thread that already has it (the main one, which WebView2 set up)
    /// keeps its own apartment
    struct Com(bool);

    impl Com {
      fn init() -> Com {
        // SAFETY: balanced by CoUninitialize in Drop when this call initialized COM (S_OK or S_FALSE)
        let hr = unsafe { CoInitializeEx(std::ptr::null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32) };
        Com(hr >= 0)
      }
    }

    impl Drop for Com {
      fn drop(&mut self) {
        if self.0 {
          // SAFETY: balances the CoInitializeEx that succeeded on this thread in init
          unsafe { CoUninitialize() };
        }
      }
    }

    pub fn open(target: &str) -> Result<(), String> {
      let _com = Com::init();
      let (verb, file) = (wide("open"), wide(target));
      // SAFETY: a zeroed SHELLEXECUTEINFOW is valid; the strings outlive the call
      let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
      info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
      // Synchronous (the call returns once the handler has the target), no error dialog: the page shows the error
      info.fMask = SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
      info.lpVerb = verb.as_ptr();
      info.lpFile = file.as_ptr();
      info.nShow = SW_SHOWNORMAL;
      // SAFETY: `info` is initialized above and the strings it points at outlive the call
      if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(format!("{target}: {}", std::io::Error::last_os_error()));
      }
      Ok(())
    }

    /// An Explorer window on the item's folder with the item selected
    pub fn reveal(path: &str) -> Result<(), String> {
      let _com = Com::init();
      let w = wide(&path.replace('/', "\\"));
      // SAFETY: the item ID list is freed below; a null one means no such item
      let pidl = unsafe { ILCreateFromPathW(w.as_ptr()) };
      if pidl.is_null() {
        return Err(format!("{path}: not found"));
      }
      // SAFETY: `pidl` is the non-null list created above; no child items are passed
      let hr = unsafe { SHOpenFolderAndSelectItems(pidl, 0, std::ptr::null(), 0) };
      // SAFETY: `pidl` came from ILCreateFromPathW and is freed once
      unsafe { ILFree(pidl) };
      if hr < 0 {
        return Err(format!("{path}: {}", std::io::Error::from_raw_os_error(hr)));
      }
      Ok(())
    }
  }
}

/// The settings snapshot hello carries: the shell's own store plus, until an engine has created the shared file, the
/// shared keys found in VS Code's (or Cursor's) settings, so the first engine seeds it from the IDE's values
pub fn hello_settings(
  own: Map<String, Value>,
  shared_exists: bool,
  ide: impl FnOnce() -> Option<(PathBuf, Map<String, Value>)>,
  log: &dyn Fn(&str),
) -> Map<String, Value> {
  let mut out = own;
  if shared_exists {
    return out;
  }
  if let Some((from, values)) = ide() {
    log(&format!("shared settings will be seeded from {}", from.display()));
    out.extend(values.into_iter().filter(|(k, _)| acpira_shared::settings::is_shared_setting(k)));
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;
  use std::sync::Mutex;

  #[derive(Default)]
  struct Fake {
    calls: Mutex<Vec<String>>,
    editor_present: bool,
    opener: Option<&'static str>,
    watching: bool,
  }

  impl Fake {
    fn did(&self, s: String) {
      self.calls.lock().unwrap().push(s);
    }
    fn calls(&self) -> Vec<String> {
      self.calls.lock().unwrap().clone()
    }
  }

  impl Platform for Fake {
    fn write_setting(&self, key: &str, value: Option<Value>) {
      self.did(format!("set {key}={}", value.unwrap_or(Value::Null)));
    }
    fn setting(&self, key: &str) -> Option<Value> {
      (key == "openFilesWith").then(|| json!(self.opener.unwrap_or("system")))
    }
    fn log(&self, _: &str) {}
    fn watching(&self, _: &str) -> bool {
      self.watching
    }
    fn chime(&self) {
      self.did("chime".into());
    }
    fn notification(&self, title: &str, _: &str, session_id: &str) {
      self.did(format!("notify {title} {session_id}"));
    }
    fn toast(&self, level: &str, text: &str, _: Option<ToastOpen>) {
      self.did(format!("toast {level} {text}"));
    }
    fn open_window(&self, session_id: Option<String>) {
      self.did(format!("window {session_id:?}"));
    }
    fn open_browser(&self, url: &str, _: Option<&str>) -> Result<BrowserOpened, String> {
      Ok(BrowserOpened { url: url.into(), reused: None })
    }
    fn terminal(&self, r: TerminalRequest) {
      self.did(format!("terminal {}", r.title));
    }
    fn open_in_editor(&self, editor: Editor, target: &FileTarget) -> bool {
      self.did(format!("editor {} {}:{:?}", editor.id(), target.path, target.line));
      self.editor_present
    }
    fn open_external(&self, url: &str) -> Result<(), String> {
      self.did(format!("external {url}"));
      Ok(())
    }
    fn reveal(&self, path: &str) -> Result<(), String> {
      self.did(format!("reveal {path}"));
      Ok(())
    }
    fn open_path(&self, path: &str) -> Result<(), String> {
      self.did(format!("open {path}"));
      Ok(())
    }
    fn t(&self, key: &str, params: &[(&str, &str)]) -> String {
      format!("{key}{params:?}")
    }
  }

  fn req(v: Value) -> PlatformRequest {
    serde_json::from_value(v).unwrap()
  }

  #[test]
  fn files_open_in_the_chosen_editor_and_fall_back_to_the_default_app_with_a_toast() {
    let p = Fake { opener: Some("zed"), ..Default::default() };
    handle_request(req(json!({ "method": "openResolvedFile", "path": "/a.ts", "line": 3 })), &p).unwrap();
    assert_eq!(p.calls(), ["editor zed /a.ts:Some(3)", "toast info desktop.editorMissing[(\"editor\", \"Zed\")]", "open /a.ts"]);
    let p = Fake { opener: Some("code"), editor_present: true, ..Default::default() };
    handle_request(req(json!({ "method": "openResolvedFile", "path": "/a.ts" })), &p).unwrap();
    assert_eq!(p.calls(), ["editor code /a.ts:None"]);
    let p = Fake::default();
    handle_request(req(json!({ "method": "openResolvedFile", "path": "/a.ts" })), &p).unwrap();
    assert_eq!(p.calls(), ["open /a.ts"]);
  }

  #[test]
  fn a_markdown_plan_is_written_to_a_temp_file_first() {
    let p = Fake::default();
    handle_request(req(json!({ "method": "openPlanDocument", "target": { "markdown": "# plan" } })), &p).unwrap();
    let opened = p.calls()[0].strip_prefix("open ").unwrap().to_owned();
    assert_eq!(std::fs::read_to_string(&opened).unwrap(), "# plan");
    let _ = std::fs::remove_dir_all(std::path::Path::new(&opened).parent().unwrap());
  }

  #[test]
  fn notify_stays_quiet_for_a_watched_session_and_otherwise_chimes_and_notifies_as_asked() {
    let n = json!({ "method": "notify", "sessionId": "S", "title": "Done", "body": "b", "open": "Open", "sound": true, "system": true });
    let p = Fake { watching: true, ..Default::default() };
    handle_request(req(n.clone()), &p).unwrap();
    assert!(p.calls().is_empty());
    let p = Fake::default();
    handle_request(req(n), &p).unwrap();
    assert_eq!(p.calls(), ["chime", "notify Done S"]);
  }

  #[test]
  fn the_rest_go_to_their_shell_action_and_search_is_left_to_the_engine() {
    let p = Fake::default();
    handle_request(req(json!({ "method": "writeSetting", "key": "sessionScope", "value": "all" })), &p).unwrap();
    handle_request(req(json!({ "method": "revealInOS", "path": "/x" })), &p).unwrap();
    handle_request(req(json!({ "method": "openInEditor", "sessionId": "S" })), &p).unwrap();
    handle_request(req(json!({ "method": "runInTerminal", "title": "login", "command": "x", "args": [] })), &p).unwrap();
    assert_eq!(
      handle_request(req(json!({ "method": "openBrowser", "url": "http://127.0.0.1:3000/" })), &p).unwrap(),
      json!({ "url": "http://127.0.0.1:3000/" })
    );
    assert_eq!(p.calls(), ["set sessionScope=\"all\"", "reveal /x", "window Some(\"S\")", "terminal login"]);
    assert_eq!(
      handle_request(req(json!({ "method": "searchFiles", "query": "a" })), &p).unwrap_err(),
      "searchFiles is not supported by the desktop app"
    );
  }

  #[test]
  fn hello_seeds_shared_keys_from_the_ide_only_until_the_shared_file_exists() {
    let mut own = Map::new();
    own.insert("sessionScope".into(), json!("all"));
    let ide = || {
      let mut m = Map::new();
      m.insert("defaultAgent".into(), json!("kimi"));
      m.insert("sessionListPosition".into(), json!("right"));
      Some((PathBuf::from("/vscode/settings.json"), m))
    };
    assert_eq!(Value::Object(hello_settings(own.clone(), false, ide, &|_| {})), json!({ "sessionScope": "all", "defaultAgent": "kimi" }));
    assert_eq!(Value::Object(hello_settings(own, true, ide, &|_| {})), json!({ "sessionScope": "all" }));
  }
}
