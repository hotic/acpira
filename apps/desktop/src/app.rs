//! The desktop app's main process: one connection to the persistent engine (`acpira serve`, its own socket key), any
//! number of chat windows (each an engine view), browser panel windows (no view; this side holds their preview leases),
//! and the system side of the IDE actions; see docs/dev/desktop.md

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use acpira_desktop::browser::{BrowserHistory, BrowserPage, browser_look, normalize_preview_url, origin_of};
use acpira_desktop::client::{ClientOpts, HelloPayload, ShellView, SidecarClient, SidecarState};
use acpira_desktop::context_api::{ContextPayloadError, ContextSlots, add_context_msg, page_owner, parse_context_payload};
use acpira_desktop::editors::{Editor, FileTarget};
use acpira_desktop::locator::{LocateOpts, SidecarCommand, sidecar_commands};
use acpira_desktop::page::APP_ORIGIN;
use acpira_desktop::platform::{CAPABILITIES, Platform, handle_request, hello_settings, system};
use acpira_desktop::settings::SettingsStore;
use acpira_desktop::shared_settings::SharedSettingsFile;
use acpira_desktop::terminal::TerminalRequest;
use acpira_desktop::update::{self, Phase, UpdateState, Updater};
use acpira_desktop::window_state::{Bounds, Placement, WindowEntry, WindowStateStore, WindowsState, placeable};
use acpira_desktop::wire::Initial;
use acpira_shared::i18n::{Language, Locale, resolve_locale, translate};
use acpira_shared::sidecar::{BrowserOpened, BrowserReuse, ToastOpen};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tauri::{AppHandle, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::browser::BrowserPanelWindow;
use crate::chat_window::{ChatWindow, Watch, page_url};

pub const HELP_URL: &str = "https://github.com/hotic/acpira/issues";

static APP: OnceLock<Arc<App>> = OnceLock::new();

// The scheduled update checks: clear of the launch, then a few times a day
const UPDATE_FIRST_CHECK: Duration = Duration::from_secs(60);
const UPDATE_EVERY: Duration = Duration::from_secs(6 * 3600);

#[expect(
  clippy::expect_used,
  reason = "set in App::init during setup, before any window, menu, listener or callback that calls this exists"
)]
pub fn app() -> &'static Arc<App> {
  APP.get().expect("the app is set up")
}

/// None before setup finished (a request to the app protocol can come that early)
pub fn try_app() -> Option<&'static Arc<App>> {
  APP.get()
}

/// Where everything is: the page bundle and the picker script (the app bundle's resources, inside the Windows exe, or the
/// repository's dist/ in a checkout), the engine candidates, the data home shared with the IDE shells,
/// and the app's own data and logs (acpira_desktop::dirs)
pub struct Paths {
  /// The user's home (USERPROFILE on Windows), what the IDE settings and editor lookups start from
  pub home: PathBuf,
  pub webview: PathBuf,
  pub picker: PathBuf,
  pub engines: Vec<PathBuf>,
  pub data_home: PathBuf,
  pub user_data: PathBuf,
  pub logs: PathBuf,
  /// The app's cache (the updater's downloads and staged bundles under `updates/`)
  pub cache: PathBuf,
  /// WebView2's profile when it is not Tauri's default (%LOCALAPPDATA%\<identifier>): a smoke run's own
  pub webview_data: Option<PathBuf>,
}

impl Paths {
  pub fn resolve(identifier: &str) -> Paths {
    let dirs = acpira_desktop::dirs::this_app(identifier);
    let exe = acpira_desktop::dirs::this_exe().unwrap_or_default();
    let exe_dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let (webview, picker, engines) = if exe_dir.ends_with("Contents/MacOS") {
      let res = exe_dir.parent().unwrap_or(&exe_dir).join("Resources");
      // scripts/package.mjs puts the engine beside the shell as acpira-engine (`acpira` and the shell's `Acpira` are one
      // name on a case-insensitive volume)
      (
        res.join("webview"),
        res.join("picker.js"),
        vec![exe_dir.join("acpira-engine"), res.join("bin").join(acpira_desktop::locator::EXE)],
      )
    } else if cfg!(windows) && crate::embedded::has_page() {
      // Windows: one exe carries the page, the picker (written out for the engine, which reads it from a file) and the
      // engine, which is this exe run with an engine subcommand (main.rs)
      let picker = crate::embedded::picker_file(&dirs.cache).unwrap_or_default();
      (PathBuf::new(), picker, vec![exe.clone()])
    } else {
      // A checkout: the repository root holds dist/webview, dist/picker.js and the engine builds. It is the exe's first
      // ancestor with a dist/webview (a build copied elsewhere with the checkout), else where this crate was built
      let root = exe
        .ancestors()
        .find(|d| d.join("dist").join("webview").is_dir() && d.join("rust").is_dir())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| {
          let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
          root.canonicalize().unwrap_or(root)
        });
      // The Windows exe is its own engine; a checkout's builds come after it
      let mut engines = if cfg!(windows) { vec![exe.clone()] } else { vec![] };
      if let Some(dir) = acpira_desktop::locator::platform_dir() {
        engines.push(root.join("dist/sidecar").join(dir).join(acpira_desktop::locator::EXE));
      }
      engines.push(root.join("rust/target/debug").join(acpira_desktop::locator::EXE));
      (root.join("dist/webview"), root.join("dist/picker.js"), engines)
    };
    let data_home = std::env::var("ACPIRA_HOME")
      .ok()
      .map(|s| s.trim().to_owned())
      .filter(|s| !s.is_empty())
      .map(PathBuf::from)
      .unwrap_or_else(|| dirs.home.join(".acpira"));
    // Smoke runs and screenshots keep the app's own settings, window state and log out of the real profile
    let (user_data, logs, cache, webview_data) =
      match std::env::var("ACPIRA_DESKTOP_USER_DATA").ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()) {
        Some(dir) => {
          let dir = PathBuf::from(dir);
          (dir.clone(), dir.join("logs"), dir.join("cache"), Some(dir.join("webview")))
        }
        None => (dirs.user_data, dirs.logs, dirs.cache, None),
      };
    Paths { home: dirs.home, webview, picker, engines, data_home, user_data, logs, cache, webview_data }
  }
}

/// The Electron app's profile (~/Library/Application Support/Acpira): a first launch starts from its settings, window
/// places and browser history, so moving to this shell keeps them. There was no Electron app outside macOS
fn migrate_electron_profile(paths: &Paths, log: &dyn Fn(&str)) {
  let user_data = &paths.user_data;
  if !cfg!(target_os = "macos") || std::env::var_os("ACPIRA_DESKTOP_USER_DATA").is_some() || user_data.join("settings.json").exists() {
    return;
  }
  let old = paths.home.join("Library/Application Support/Acpira");
  if !old.join("settings.json").is_file() {
    return;
  }
  let _ = std::fs::create_dir_all(user_data);
  for f in ["settings.json", "windows.json", "browser-history.json"] {
    if std::fs::copy(old.join(f), user_data.join(f)).is_ok() {
      log(&format!("took {f} over from the Electron app's profile"));
    }
  }
}

#[derive(Default)]
pub struct ChatWindowOpen {
  pub place: Option<Placement>,
  pub maximized: bool,
  pub watch: Option<Watch>,
}

pub struct App {
  pub handle: AppHandle,
  pub paths: Paths,
  pub settings: Arc<SettingsStore>,
  pub shared: SharedSettingsFile,
  pub history: BrowserHistory,
  client: OnceLock<SidecarClient>,
  windows: Mutex<Vec<Arc<ChatWindow>>>,
  browsers: Mutex<Vec<Arc<BrowserPanelWindow>>>,
  last_focused: Mutex<Option<String>>,
  window_store: WindowStateStore,
  windows_state: Mutex<WindowsState>,
  /// Each window's entry as of the last save: a closed window's bounds cannot be read any more
  known: Mutex<HashMap<String, WindowEntry>>,
  pub quitting: AtomicBool,
  save_seq: AtomicU64,
  context: Mutex<ContextSlots>,
  login_env: Mutex<Option<HashMap<String, String>>>,
  log_file: PathBuf,
  pending_links: Mutex<Option<Vec<String>>>,
  failure_shown: AtomicBool,
  window_seq: AtomicU64,
  pub background: bool,
  notifier: OnceLock<Option<crate::os::Notifier>>,
  updater: OnceLock<Arc<Updater>>,
  /// The updater's phase as the menu and the toast last saw it
  update_phase: Mutex<Phase>,
  /// The menu bar as the page draws it where there is no global one (Windows; menu.rs install), sent to each page on
  /// ready and on every change
  page_menu: Mutex<Option<Value>>,
}

pub fn log(line: &str) {
  let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
  let stamped = format!("{} {line}", acpira_shared::time::iso_of_ms(ms));
  eprintln!("{stamped}");
  if let Some(a) = APP.get() {
    use std::io::Write;
    let _ = std::fs::create_dir_all(&a.paths.logs);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&a.log_file) {
      let _ = writeln!(f, "{stamped}");
    }
  }
}

fn log_arc() -> Arc<dyn Fn(&str) + Send + Sync> {
  Arc::new(log)
}

impl App {
  pub fn init(handle: AppHandle, paths: Paths, links: Vec<String>) -> Arc<App> {
    migrate_electron_profile(&paths, &log);
    let settings = Arc::new(SettingsStore::new(paths.user_data.join("settings.json")));
    let shared = SharedSettingsFile::new(&paths.data_home);
    let history = BrowserHistory { file: paths.user_data.join("browser-history.json") };
    let window_store = WindowStateStore::new(paths.user_data.join("windows.json"));
    let windows_state = window_store.read();
    let log_file = paths.logs.join("desktop.log");
    let background = std::env::var("ACPIRA_DESKTOP_BACKGROUND").as_deref() == Ok("1");
    let app = Arc::new(App {
      handle,
      paths,
      settings,
      shared,
      history,
      client: OnceLock::new(),
      windows: Mutex::new(vec![]),
      browsers: Mutex::new(vec![]),
      last_focused: Mutex::new(None),
      window_store,
      windows_state: Mutex::new(windows_state),
      known: Mutex::new(HashMap::new()),
      quitting: AtomicBool::new(false),
      save_seq: AtomicU64::new(0),
      context: Mutex::new(ContextSlots::default()),
      login_env: Mutex::new(None),
      log_file,
      pending_links: Mutex::new(Some(links)),
      failure_shown: AtomicBool::new(false),
      window_seq: AtomicU64::new(0),
      background,
      notifier: OnceLock::new(),
      updater: OnceLock::new(),
      update_phase: Mutex::new(Phase::Idle),
      page_menu: Mutex::new(None),
    });
    let _ = APP.set(app.clone());
    app.register_links();
    log(&format!(
      "acpira desktop {} (tauri) data {} profile {}",
      env!("CARGO_PKG_VERSION"),
      app.paths.data_home.display(),
      app.paths.user_data.display()
    ));
    let _ = app.client.set(app.make_client());
    let updater = app.make_updater();
    // A check a minute after launch, then every six hours; the smoke run drives its checks itself so no toast lands
    // in the middle of its other steps
    if !cfg!(feature = "smoke") {
      updater.schedule(UPDATE_FIRST_CHECK, UPDATE_EVERY);
    }
    let _ = app.updater.set(updater);
    let client = app.client().clone();
    app.settings.on_change(move |keys, snapshot| {
      client.event(json!({ "type": "settingsChanged", "keys": keys, "settings": snapshot }));
    });
    // The menu's language and the browser panel's look follow the shared file, whichever engine wrote it
    app.shared.watch(Duration::from_secs(1), |keys| {
      let app = crate::app::app();
      if keys.iter().any(|k| k == "language" || k == "desktopLayout") {
        app.refresh_menu();
      }
      if keys.iter().any(|k| acpira_desktop::browser::affects_look(k)) {
        for b in app.browsers.lock().iter() {
          b.relay.post_look();
        }
        for w in app.windows.lock().iter() {
          w.post_browser_look();
        }
      }
    });
    app
  }

  /// Windows: acpira:// links open this exe and toasts carry the app's name and icon (HKCU, rewritten when the exe moved
  /// or was replaced). A smoke run, with a profile of its own, leaves the user's registry alone
  fn register_links(&self) {
    #[cfg(windows)]
    if self.paths.webview_data.is_none()
      && let Ok(exe) = std::env::current_exe()
    {
      match acpira_desktop::registry::ensure(&acpira_desktop::registry::link_handler(&exe)) {
        Ok(0) => {}
        Ok(n) => log(&format!("acpira:// links now open {} ({n} registry values)", exe.display())),
        Err(e) => log(&format!("acpira:// link registration: {e}")),
      }
      crate::win::register_identity(&self.paths.cache, log_arc());
    }
  }

  /// The app's own updater (docs/dev/desktop.md "Updates"): a release build's configuration, or the smoke run's local
  /// release server; what keeps this install from replacing itself is known before any check
  fn make_updater(&self) -> Arc<Updater> {
    let exe = acpira_desktop::dirs::this_exe();
    // The bundle on macOS, the packaged exe itself on Windows (none for a cargo build)
    let installed = exe.as_deref().and_then(update::swap::install_of);
    let (config, manual) = crate::smoke::update_config(installed.as_deref())
      .unwrap_or_else(|| (update::Config::release(), update::install_blocker(installed.as_deref(), update::stage::signing_team)));
    let env = update::Env {
      config,
      installed,
      cache: self.paths.cache.join("updates"),
      current: env!("CARGO_PKG_VERSION").into(),
      os_version: update::os_version(),
      manual,
      // The engine's route: ACPIRA_PROXY, else the shared `proxy` setting
      proxy: Box::new(|| {
        use acpira_shared::proxy::{AUTO_PORT, ProxyMode, port_listening, resolve};
        let setting = app().setting("proxy");
        resolve(&ProxyMode::effective(setting.as_ref().and_then(Value::as_str)), || port_listening(AUTO_PORT))
      }),
      log: log_arc(),
    };
    let auto = self.settings.get("updateAuto") != Some(Value::Bool(false));
    Updater::new(env, auto, |s| {
      let s = s.clone();
      let h = app().handle.clone();
      let _ = h.run_on_main_thread(move || app().update_changed(s));
    })
  }

  pub fn updater(&self) -> Option<&Arc<Updater>> {
    self.updater.get()
  }

  /// Every page gets the new state; reaching Ready (or leaving it) rebuilds the menu's item, and Ready says so in the
  /// page used last with a restart button. Main thread
  fn update_changed(&self, s: UpdateState) {
    self.broadcast(json!({ "type": "updateState", "state": s }));
    let before = std::mem::replace(&mut *self.update_phase.lock(), s.phase);
    if (before == Phase::Ready) != (s.phase == Phase::Ready) {
      self.refresh_menu();
    }
    if s.phase == Phase::Ready && before != Phase::Ready {
      let version = s.version.clone().unwrap_or_default();
      let text = self.t("desktop.update.ready", &[("version", &version)]);
      log(&format!("toast info: {text}"));
      match self.focused_window() {
        Some(w) => w.post(json!({ "type": "toast", "level": "info", "text": text,
          "action": { "label": self.t("desktop.update.restart", &[]), "command": "updateInstall" } })),
        None => self.system_notification("Acpira", &text, None),
      }
    }
  }

  /// A check from the page or the menu; it downloads when the switch is on or the page asked for it
  pub fn check_updates(&self, download: bool) {
    if let Some(u) = self.updater() {
      u.check(download || u.state().auto);
    }
  }

  pub fn set_update_auto(&self, on: bool) {
    self.settings.set("updateAuto", Some(Value::Bool(on)));
    if let Some(u) = self.updater() {
      u.set_auto(on);
    }
  }

  /// Restart to update: the staged bundle takes the installed one's place, the app quits as usual (window places
  /// saved, the engine left running its turns), and a helper starts the new bundle once this process is gone (on
  /// Windows the new exe itself, which waits for this process before claiming the single instance)
  pub fn restart_to_update(&self) {
    let Some(u) = self.updater() else { return };
    let Some(installed) = u.installed().map(Path::to_path_buf) else { return };
    if let Err(e) = u.apply() {
      self.toast("error", &e, None);
      return;
    }
    self.before_quit();
    if let Err(e) = update::swap::relaunch_after_exit(std::process::id(), &installed, crate::smoke::RELAUNCH_DIRECT) {
      log(&format!("update: the relaunch helper did not start: {e}"));
    }
    self.handle.exit(0);
  }

  #[expect(clippy::expect_used, reason = "set in App::init right after APP, before any window or callback that calls this exists")]
  pub fn client(&self) -> &SidecarClient {
    self.client.get().expect("the client is made at init")
  }

  /// A setting as the shell sees it: shared keys from <ACPIRA_HOME>/settings.json once it exists, its own otherwise
  pub fn setting(&self, key: &str) -> Option<Value> {
    self.shared.get(key, |k| self.settings.get(k))
  }

  pub fn host_language(&self) -> String {
    crate::os::preferred_language().unwrap_or_else(|| "en".into())
  }

  pub fn locale(&self) -> Locale {
    let language = self.setting("language").and_then(|v| v.as_str().and_then(Language::parse));
    resolve_locale(language, &self.host_language())
  }

  pub fn t(&self, key: &str, params: &[(&str, &str)]) -> String {
    translate(self.locale(), key, params)
  }

  pub fn browser_look(&self) -> Value {
    browser_look(&|k| self.setting(k), self.locale().tag())
  }

  fn make_client(self: &Arc<Self>) -> SidecarClient {
    let engines = self.paths.engines.clone();
    let data_home = self.paths.data_home.clone();
    let commands = move || {
      let over = std::env::var("ACPIRA_SIDECAR_BIN").ok();
      let env = app().login_env.lock().clone().unwrap_or_default();
      sidecar_commands(&LocateOpts { candidates: &engines, override_bin: over.as_deref() })
        .into_iter()
        .map(|mut c: SidecarCommand| {
          let mut merged: std::collections::BTreeMap<String, String> = env.clone().into_iter().collect();
          merged.extend(std::mem::take(&mut c.env));
          c.env = merged;
          c
        })
        .collect()
    };
    let hello = || {
      let a = app();
      let paths = acpira_desktop::vscode_settings::ide_settings_paths(
        acpira_desktop::editors::this_platform(),
        &|k| std::env::var(k).ok(),
        &a.paths.home,
      );
      HelloPayload {
        client: json!({ "name": "acpira-desktop", "version": env!("CARGO_PKG_VERSION"), "capabilities": CAPABILITIES }),
        env: json!({ "hostLanguage": a.host_language(), "projects": true, "blobBase": format!("{APP_ORIGIN}/blobs") }),
        settings: hello_settings(
          a.settings.snapshot(),
          a.shared.exists(),
          || acpira_desktop::vscode_settings::read_ide_settings(&paths, &log),
          &log,
        ),
      }
    };
    let on_request = |r| -> acpira_desktop::client::RequestFuture {
      Box::pin(async move {
        // The actions touch windows and AppKit: they run on the main thread, the answer comes back here
        let (tx, rx) = tokio::sync::oneshot::channel();
        let handle = app().handle.clone();
        let sent = handle.run_on_main_thread(move || {
          let _ = tx.send(handle_request(r, &**app()));
        });
        if sent.is_err() {
          return Err("the app is quitting".to_owned());
        }
        rx.await.unwrap_or_else(|_| Err("the app is quitting".to_owned()))
      })
    };
    let mut opts = ClientOpts::new(commands, hello, on_request, log_arc());
    // The persistent engine (a Unix socket, a named pipe on Windows); ACPIRA_ENGINE=stdio runs it attached instead
    {
      opts.engine = Box::new(move |cmd| {
        if std::env::var("ACPIRA_ENGINE").as_deref() == Ok("stdio") {
          return None;
        }
        Some(acpira_desktop::engine::engine_endpoint(&acpira_desktop::engine::EndpointOpts {
          binary: &cmd.command,
          home: &data_home,
          cwd: None,
          shell: Some("desktop"),
          tmp: None,
          uid: None,
        }))
      });
    }
    opts.on_state = Box::new(|state, detail| {
      if state != SidecarState::Failed {
        return;
      }
      let detail = detail.unwrap_or_default().to_owned();
      log(&format!("engine failed: {detail}"));
      app().engine_failed(detail);
    });
    SidecarClient::new(opts, tauri::async_runtime::handle().inner().clone())
  }

  /// A failure dialog at a time; the client's own restarts come first, this is what follows when they gave up
  fn engine_failed(&self, detail: String) {
    if self.background || self.failure_shown.swap(true, Ordering::SeqCst) {
      return;
    }
    let (title, retry, show, close) =
      (self.t("desktop.engine.failed", &[]), self.t("common.retry", &[]), self.t("desktop.menu.showLog", &[]), self.t("common.close", &[]));
    tauri::async_runtime::spawn(async move {
      let answer = rfd::AsyncMessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(&title)
        .set_description(&detail)
        .set_buttons(rfd::MessageButtons::YesNoCancelCustom(retry.clone(), show.clone(), close))
        .show()
        .await;
      let a = app();
      a.failure_shown.store(false, Ordering::SeqCst);
      match answer {
        rfd::MessageDialogResult::Custom(c) if c == retry => a.client().retry(),
        rfd::MessageDialogResult::Custom(c) if c == show => a.show_log(),
        _ => {}
      }
    });
  }

  /// The login shell's environment (read off the main thread while the first window loads), then the engine
  pub fn start_engine(self: &Arc<Self>) {
    std::thread::spawn(|| {
      let env = acpira_desktop::login_env::resolve_login_env(&log, None);
      let a = app();
      *a.login_env.lock() = env;
      a.client().start();
    });
  }

  pub fn windows(&self) -> Vec<Arc<ChatWindow>> {
    self.windows.lock().clone()
  }

  pub fn window_by_label(&self, label: &str) -> Option<Arc<ChatWindow>> {
    self.windows.lock().iter().find(|w| w.label == label).cloned()
  }

  pub fn browser_by_label(&self, label: &str) -> Option<Arc<BrowserPanelWindow>> {
    self.browsers.lock().iter().find(|w| w.label == label).cloned()
  }

  pub fn focused_window(&self) -> Option<Arc<ChatWindow>> {
    let last = self.last_focused.lock().clone();
    let windows = self.windows.lock();
    last.and_then(|l| windows.iter().find(|w| w.label == l).cloned()).or_else(|| windows.first().cloned())
  }

  pub fn on_focus(&self, w: &ChatWindow) {
    *self.last_focused.lock() = Some(w.label.clone());
    self.client().event(json!({ "type": "windowFocus" }));
  }

  /// The page has its init state: the live context slots go after
  pub fn on_page_ready(&self, w: &ChatWindow) {
    if let Some(u) = self.updater() {
      w.post(json!({ "type": "updateState", "state": u.state() }));
    }
    if let Some(menus) = self.page_menu.lock().clone() {
      w.post(json!({ "type": "desktopMenu", "menus": menus }));
      w.post(json!({ "type": "windowState", "maximized": w.window.is_maximized().unwrap_or(false) }));
    }
    if self.share_context() {
      for m in self.context.lock().replay() {
        w.post(m);
      }
    }
  }

  /// Where WebView2 keeps its profile: Tauri's default (%LOCALAPPDATA%\<identifier>) unless a smoke run gave the app a
  /// profile of its own; every webview of the process has to use the same one. WKWebView keeps its data in the system's
  /// store whatever this says (Tauri only keys the webviews' shared context by it)
  fn webview_data(&self) -> PathBuf {
    self.paths.webview_data.clone().unwrap_or_else(|| {
      if cfg!(windows) { self.paths.logs.parent().map(Path::to_path_buf).unwrap_or_default() } else { self.paths.user_data.clone() }
    })
  }

  /// WebView2's browser arguments (the same for every webview of the process): wry's own (no mini menu, no SmartScreen
  /// check of the app's pages) and, in a background run, rendering that goes on while other windows cover the app's
  fn browser_args(&self) -> String {
    let mut args = String::from("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection");
    if self.background {
      args.push_str(",CalculateNativeWinOcclusion --disable-backgrounding-occluded-windows --disable-renderer-backgrounding");
      args.push_str(" --disable-background-timer-throttling");
    }
    args
  }

  fn dark(&self) -> bool {
    crate::os::dark_mode()
  }

  fn next_label(&self, prefix: &str) -> String {
    format!("{prefix}-{}", self.window_seq.fetch_add(1, Ordering::SeqCst) + 1)
  }

  /// A new chat window on a session, the most recent one, or none (a fresh view)
  pub fn open(self: &Arc<Self>, initial: Option<Initial>, how: ChatWindowOpen) -> Option<Arc<ChatWindow>> {
    let label = self.next_label("chat");
    let mut b = WebviewWindowBuilder::new(&self.handle, &label, first_url("editor"))
      .title("Acpira")
      .inner_size(1180.0, 820.0)
      .min_inner_size(420.0, 360.0)
      .visible(false)
      .background_color(if self.dark() {
        tauri::window::Color(0x18, 0x18, 0x18, 0xff)
      } else {
        tauri::window::Color(0xf8, 0xf8, 0xf8, 0xff)
      })
      .initialization_script(crate::INIT_SCRIPT.as_str())
      .data_directory(self.webview_data())
      .additional_browser_args(&self.browser_args())
      // The page's own drag and drop: Tauri's handler would take every drag (Finder files, the session list's reordering)
      // away from the DOM; the page reads a dropped file itself and asks for its path (acpira_drag_paths)
      .disable_drag_drop_handler()
      .on_navigation(allowed_navigation)
      .on_new_window(|url, _| {
        open_outside(url.as_str());
        tauri::webview::NewWindowResponse::Deny
      });
    match how.place {
      Some(Placement::At(r)) => b = b.position(r.x, r.y).inner_size(r.width, r.height),
      Some(Placement::Size { width, height }) => b = b.inner_size(width, height),
      None => {}
    }
    #[cfg(target_os = "macos")]
    {
      // No title bar: the page's column title row and sheet header are the title bar (data-drag-region) and leave the
      // traffic lights their room (desktop.css --lights-w). Tauri measures the lights 9 pt lower than Electron's
      // trafficLightPosition, so (17, 32) here is Electron's (17, 23): the lights' centre on the header row's centre line
      b =
        b.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true).traffic_light_position(tauri::LogicalPosition::new(17.0, 32.0));
    }
    // No system title bar either: the page draws the caption's buttons and the menu (chat/DesktopFrame.tsx WindowCaption,
    // AppMenus). The system still gives the undecorated window its shadow, rounded corners and resize edges, and the
    // title row's drag and double click are the system's own (init.js)
    #[cfg(windows)]
    {
      b = b.decorations(false).shadow(true);
    }
    let window = match b.build() {
      Ok(w) => w,
      Err(e) => {
        log(&format!("window failed: {e}"));
        return None;
      }
    };
    self.prepare(&window);
    let w = Arc::new(ChatWindow::new(window.clone(), initial, how.watch));
    self.windows.lock().push(w.clone());
    {
      let mut last = self.last_focused.lock();
      if last.is_none() {
        *last = Some(label.clone());
      }
    }
    if how.maximized {
      let _ = window.maximize();
    }
    // Attach before the page loads: its `ready` must find the view
    self.client().attach(w.clone() as Arc<dyn ShellView>);
    if cfg!(target_os = "macos") {
      w.load();
    }
    self.schedule_save();
    Some(w)
  }

  /// What every page's webview gets before its first real page: the frame-rate switch, background rendering for a
  /// smoke run, and on Windows the clipboard and dropped paths (win::hook_webview)
  fn prepare(&self, window: &WebviewWindow) {
    crate::os::unlock_frame_rate(window, log_arc());
    #[cfg(windows)]
    crate::win::hook_webview(window, log_arc());
    if self.background {
      crate::os::keep_rendering_in_background(window, log_arc());
    }
  }

  /// The page listens: the window shows (the Electron app's ready-to-show), without the focus in a background run
  pub fn page_attached(&self, label: &str) {
    let window = self.window_by_label(label).map(|w| w.window.clone()).or_else(|| self.browser_by_label(label).map(|b| b.window.clone()));
    let Some(window) = window else { return };
    if window.is_visible().unwrap_or(true) {
      return;
    }
    if self.background {
      crate::os::order_back(&window);
    } else if self.browser_by_label(label).is_some_and(|b| !b.focus_on_show()) {
      crate::os::order_front(&window);
    } else {
      let _ = window.show();
      let _ = window.set_focus();
    }
  }

  /// View › Reload: the page starts over on the app's own page
  pub fn reload(&self, label: &str) {
    if let Some(w) = self.window_by_label(label) {
      w.load();
    } else if let Some(b) = self.browser_by_label(label) {
      b.transport.detach();
      let _ = b.window.navigate(page_url("browser"));
    }
  }

  pub fn window_closed(&self, label: &str) {
    if let Some(b) = self.browser_by_label(label) {
      b.relay.dispose();
      self.browsers.lock().retain(|x| x.label != label);
      return;
    }
    let Some(w) = self.window_by_label(label) else { return };
    self.client().detach(&w.view_id);
    w.dispose();
    self.windows.lock().retain(|x| x.label != label);
    {
      let mut last = self.last_focused.lock();
      if last.as_deref() == Some(label) {
        *last = None;
      }
    }
    // A window closed by hand lends its place to the next new one; at quit every window stays in the list
    let entry = self.known.lock().remove(label);
    if !self.quitting.load(Ordering::SeqCst)
      && let Some(e) = entry
    {
      self.windows_state.lock().last = Some(e);
    }
    self.save_windows();
  }

  /// The freshest entry when a window is about to close (the close button, ⌘W)
  pub fn window_closing(&self, label: &str) {
    if self.quitting.load(Ordering::SeqCst) {
      return;
    }
    if let Some(w) = self.window_by_label(label)
      && let Some(e) = self.entry_of(&w)
    {
      self.known.lock().insert(label.to_owned(), e);
    }
  }

  fn entry_of(&self, w: &ChatWindow) -> Option<WindowEntry> {
    let win = &w.window;
    let maximized = win.is_maximized().unwrap_or(false);
    let scale = win.scale_factor().ok()?;
    let bounds = if maximized || win.is_fullscreen().unwrap_or(false) || win.is_minimized().unwrap_or(false) {
      // Not the normal bounds: keep the ones saved before
      self.known.lock().get(&w.label).and_then(|e| e.bounds)
    } else {
      let pos = win.outer_position().ok()?.to_logical::<f64>(scale);
      let size = win.inner_size().ok()?.to_logical::<f64>(scale);
      Some(Bounds { x: pos.x, y: pos.y, width: size.width, height: size.height })
    };
    Some(WindowEntry { bounds, maximized, session: w.session_id() })
  }

  pub fn save_windows(&self) {
    if self.quitting.load(Ordering::SeqCst) {
      return;
    }
    let live = self.windows();
    let entries: Vec<WindowEntry> = live
      .iter()
      .filter_map(|w| {
        let e = self.entry_of(w)?;
        self.known.lock().insert(w.label.clone(), e.clone());
        Some(e)
      })
      .collect();
    let mut state = self.windows_state.lock();
    state.windows = entries;
    self.window_store.write(&state);
  }

  /// Moved, resized or on another session: saved once things settle
  pub fn schedule_save(&self) {
    let seq = self.save_seq.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
      tokio::time::sleep(Duration::from_millis(500)).await;
      let a = app();
      if a.save_seq.load(Ordering::SeqCst) == seq {
        let h = a.handle.clone();
        let _ = h.run_on_main_thread(|| app().save_windows());
      }
    });
  }

  pub fn place_of(&self, e: Option<&WindowEntry>) -> ChatWindowOpen {
    let Some(e) = e else { return ChatWindowOpen::default() };
    let areas: Vec<Bounds> = self
      .handle
      .available_monitors()
      .unwrap_or_default()
      .iter()
      .map(|m| {
        let s = m.scale_factor();
        let a = m.work_area();
        Bounds {
          x: f64::from(a.position.x) / s,
          y: f64::from(a.position.y) / s,
          width: f64::from(a.size.width) / s,
          height: f64::from(a.size.height) / s,
        }
      })
      .collect();
    ChatWindowOpen { place: e.bounds.map(|b| placeable(b, &areas)), maximized: e.maximized, watch: None }
  }

  fn place_of_last(&self) -> ChatWindowOpen {
    let last = self.windows_state.lock().last.clone();
    self.place_of(last.as_ref())
  }

  /// The windows open at the last quit come back; links that launched the app show their sessions after that
  pub fn restore(self: &Arc<Self>) {
    let state = self.windows_state.lock().clone();
    let restore = if state.windows.is_empty() { vec![state.last.clone().unwrap_or_default()] } else { state.windows.clone() };
    for e in &restore {
      let initial = Some(e.session.clone().map(Initial::Session).unwrap_or(Initial::MostRecent));
      self.open(initial, self.place_of(Some(e)));
    }
    let links = self.pending_links.lock().take().unwrap_or_default();
    for l in links {
      self.take_link(&l);
    }
  }

  /// A notification or a session link: the window already showing it, else the focused one switches, else a new one
  pub fn show_session(self: &Arc<Self>, id: &str) {
    if let Some(on) = self.windows().into_iter().find(|w| w.session_id().as_deref() == Some(id)) {
      on.reveal();
      return;
    }
    if let Some(w) = self.focused_window() {
      w.reveal();
      w.post(json!({ "type": "showSession", "id": id }));
      return;
    }
    self.open(Some(Initial::Session(id.to_owned())), self.place_of_last());
  }

  /// Session links (acpira://session/<id>), from the launch arguments, a second launch or the system. A link names a
  /// session this machine's store has (or a window shows), else the page says the link came from elsewhere
  pub fn take_link(self: &Arc<Self>, url: &str) {
    {
      let mut pending = self.pending_links.lock();
      if let Some(p) = pending.as_mut() {
        p.push(url.to_owned());
        return;
      }
    }
    let Some(id) = acpira_shared::session_links::session_id_of_link(url) else {
      // Any other acpira:// link (a plain notification's click on Windows, `acpira://open`) brings the app forward
      log(&format!("link without a session: {url}"));
      self.second_instance(vec![]);
      return;
    };
    let sessions = self.client().sessions_dir().map(PathBuf::from).unwrap_or_else(|| self.paths.data_home.join("sessions"));
    let known = self.windows().iter().any(|w| w.session_id().as_deref() == Some(id.as_str()))
      || sessions.join(format!("{id}.json")).exists()
      || self.paths.data_home.join("bridges/chatgpt").join(format!("{id}.json")).exists();
    if known {
      self.show_session(&id);
      return;
    }
    let Some(w) = self.focused_window().or_else(|| self.open(Some(Initial::MostRecent), self.place_of_last())) else { return };
    w.reveal();
    w.post(json!({ "type": "toast", "level": "error", "text": self.t("sessionLink.notHere", &[]) }));
  }

  /// Another launch's arguments: its links, else the app comes forward (a new window when none is open)
  pub fn second_instance(self: &Arc<Self>, args: Vec<String>) {
    let links = acpira_desktop::single_instance::links_in(&args);
    if !links.is_empty() {
      for l in links {
        self.take_link(&l);
      }
      return;
    }
    match self.focused_window() {
      Some(w) => w.reveal(),
      None => {
        self.open(Some(Initial::MostRecent), self.place_of_last());
      }
    }
  }

  /// The Dock icon clicked with no window open
  #[cfg(target_os = "macos")]
  pub fn reopen(self: &Arc<Self>) {
    if self.windows().is_empty() {
      self.open(Some(Initial::MostRecent), self.place_of_last());
    }
  }

  /// A toast lands in the page used last; with no window open it becomes a system notification
  pub fn toast(&self, level: &str, text: &str, open: Option<ToastOpen>) {
    log(&format!("toast {level}: {text}"));
    if let Some(w) = self.focused_window() {
      let mut m = json!({ "type": "toast", "level": level, "text": text });
      if let Some(o) = open {
        m["open"] = json!(o);
      }
      w.post(m);
      return;
    }
    self.system_notification("Acpira", text, None);
  }

  pub(crate) fn system_notification(&self, title: &str, body: &str, session: Option<&str>) {
    // A background run (smoke) shows nothing on the screen: no banner, and no permission prompt for the first one
    if self.background {
      log(&format!("notification (background run, not shown): {title}: {body}"));
      return;
    }
    let notifier = self.notifier.get_or_init(|| crate::os::Notifier::new(crate::app::session_clicked, log_arc()));
    if let Some(n) = notifier {
      n.show(title, body, session);
    }
  }

  fn share_context(&self) -> bool {
    self.settings.get("shareEditorSelection") != Some(Value::Bool(false))
  }

  /// The menu the pages draw changed (menu.rs install, Windows)
  pub fn set_page_menu(&self, menus: Value) {
    *self.page_menu.lock() = Some(menus.clone());
    self.broadcast(json!({ "type": "desktopMenu", "menus": menus }));
  }

  /// The page menu's top-level titles (the smoke's menuBar outside macOS)
  #[cfg(feature = "smoke")]
  pub fn page_menu_titles(&self) -> Option<Vec<String>> {
    let menus = self.page_menu.lock().clone()?;
    Some(menus.as_array()?.iter().filter_map(|m| m["label"].as_str().map(str::to_owned)).collect())
  }

  /// A window was resized: its page shows the caption's maximize or restore glyph (Windows)
  pub fn window_resized(&self, label: &str) {
    if self.page_menu.lock().is_none() {
      return;
    }
    if let Some(w) = self.window_by_label(label) {
      let maximized = w.window.is_maximized().unwrap_or(false);
      if w.maximized.swap(maximized, std::sync::atomic::Ordering::SeqCst) != maximized {
        w.post(json!({ "type": "windowState", "maximized": maximized }));
      }
    }
  }

  fn broadcast(&self, m: Value) {
    for w in self.windows() {
      w.post(m.clone());
    }
  }

  fn publish_context(&self, m: Option<Value>) {
    if let Some(m) = m
      && self.share_context()
    {
      self.broadcast(m);
    }
  }

  /// Picked elements go to the chat used last, like another tool's context; the chat comes up with them
  fn add_to_chat(self: &Arc<Self>, m: Value) {
    let Some(w) = self.focused_window().or_else(|| self.open(Some(Initial::MostRecent), self.place_of_last())) else { return };
    w.reveal();
    w.post(m);
  }

  pub fn add_picked(self: &Arc<Self>, item: Value) {
    match parse_context_payload(&json!({ "source": { "id": "browser", "name": self.t("browser.source", &[]) }, "items": [item] })) {
      Ok(p) => self.add_to_chat(add_context_msg(&p)),
      Err(e) => self.toast("error", &e.0, None),
    }
  }

  /// A page's own window.acpira.context call: what `set` resolves to (whether it shows), or the refused field
  pub fn page_context(self: &Arc<Self>, panel_id: &str, op: &str, raw: &Value) -> Result<Option<bool>, String> {
    let p = parse_context_payload(raw).map_err(|ContextPayloadError(e)| e)?;
    if op == "add" {
      if p.items.is_empty() {
        return Err("acpira.context: add needs at least one item".into());
      }
      self.add_to_chat(add_context_msg(&p));
      return Ok(None);
    }
    let owner = page_owner(panel_id, p.owner.as_deref());
    let m = self.context.lock().set(acpira_desktop::context_api::ContextPayload { owner: Some(owner), ..p });
    self.publish_context(m);
    Ok(Some(self.share_context()))
  }

  pub fn leave_page(&self, panel_id: &str) {
    let gone = self.context.lock().release_page(panel_id);
    for m in gone {
      self.publish_context(Some(m));
    }
  }

  pub fn visited(&self, page: BrowserPage) {
    self.history.visited(page);
    for b in self.browsers.lock().iter() {
      b.relay.post_history();
    }
    for w in self.windows() {
      w.post_browser_history();
    }
  }

  /// A loopback page in a browser panel window (None: an empty one on its address bar), the window already showing its
  /// site brought up (navigated when the page differs) or a new one; `focus` false for an agent's open_browser
  fn show_preview(self: &Arc<Self>, url: Option<&str>, focus: bool, folder: Option<&str>) -> Option<BrowserReuse> {
    let origin = url.and_then(origin_of);
    let shown = self
      .browsers
      .lock()
      .iter()
      .find(|b| match &origin {
        Some(o) => b.target().and_then(|t| origin_of(&t)).as_ref() == Some(o) && b.folder().as_deref() == folder,
        None => b.target().is_none(),
      })
      .cloned();
    if let Some(b) = shown {
      b.reveal(focus);
      let Some(url) = url else { return Some(BrowserReuse::Revealed) };
      if b.target().as_deref() == Some(url) {
        return Some(BrowserReuse::Revealed);
      }
      b.relay.navigate(url.to_owned());
      return Some(BrowserReuse::Navigated);
    }
    let label = self.next_label("browser");
    let title = url.map(acpira_desktop::browser::host_of).unwrap_or_else(|| "Acpira".into());
    let built = WebviewWindowBuilder::new(&self.handle, &label, first_url("browser"))
      .title(title)
      .inner_size(1200.0, 860.0)
      .min_inner_size(480.0, 360.0)
      .visible(false)
      .focused(focus)
      .background_color(if self.dark() {
        tauri::window::Color(0x18, 0x18, 0x18, 0xff)
      } else {
        tauri::window::Color(0xf8, 0xf8, 0xf8, 0xff)
      })
      .initialization_script(crate::INIT_SCRIPT.as_str())
      .data_directory(self.webview_data())
      .additional_browser_args(&self.browser_args())
      // The page's own drag and drop: Tauri's handler would take every drag (Finder files, the session list's reordering)
      // away from the DOM; the page reads a dropped file itself and asks for its path (acpira_drag_paths)
      .disable_drag_drop_handler()
      .on_navigation(allowed_navigation)
      .on_new_window(|url, _| {
        open_outside(url.as_str());
        tauri::webview::NewWindowResponse::Deny
      })
      .build();
    let window = match built {
      Ok(w) => w,
      Err(e) => {
        log(&format!("browser window failed: {e}"));
        return None;
      }
    };
    self.prepare(&window);
    let b = Arc::new(BrowserPanelWindow::new(window.clone(), folder.map(str::to_owned), focus));
    self.browsers.lock().push(b.clone());
    if cfg!(target_os = "macos") {
      let _ = window.navigate(page_url("browser"));
    }
    if let Some(u) = url {
      b.relay.navigate(u.to_owned());
    }
    None
  }

  /// open_browser: a loopback http page in a browser tab of the chat used last (its tool panel opens on it), or a window
  /// of its own with no chat open, where elements can be picked into the chat; any other http(s) page in the system
  /// browser, which the proxy cannot reach
  pub fn open_browser(self: &Arc<Self>, url: &str, folder: Option<&str>) -> Result<BrowserOpened, String> {
    let page = normalize_preview_url(url);
    let w = self.focused_window();
    if let (Some(page), Some(w)) = (&page, &w)
      && w.ready()
    {
      let reused = w.show_in_panel(page, folder).map(|r| if r == "revealed" { BrowserReuse::Revealed } else { BrowserReuse::Navigated });
      return Ok(BrowserOpened { url: page.clone(), reused });
    }
    if let Some(page) = page {
      let reused = self.show_preview(Some(&page), false, folder);
      return Ok(BrowserOpened { url: page, reused });
    }
    if !(url.starts_with("http://") || url.starts_with("https://")) {
      return Err(format!("only http(s) pages open in a browser: {url}"));
    }
    system::open_external(url)?;
    Ok(BrowserOpened { url: url.to_owned(), reused: None })
  }

  pub fn show_log(&self) {
    let _ = system::open_any(&self.log_file.to_string_lossy());
  }

  /// The menu bar's page commands go to the chat used last (a new window when none is open)
  pub fn command(self: &Arc<Self>, c: &str) {
    let Some(w) = self.focused_window().or_else(|| self.open(Some(Initial::MostRecent), self.place_of_last())) else { return };
    w.reveal();
    w.post(json!({ "type": "shellCommand", "command": c }));
  }

  /// A new window bound to a fresh ChatGPT mirror; its connection prompt goes to the clipboard once the window shows it
  pub fn connect_chatgpt(self: &Arc<Self>) {
    let watch: Watch = Box::new(|m| {
      if !matches!(m.kind.as_str(), "session" | "init") {
        return false;
      }
      let Ok(v) = serde_json::from_str::<Value>(m.raw.get()) else { return false };
      let s = if m.kind == "session" { &v["session"] } else { &v["state"]["active"] };
      let Some(prompt) = s["external"]["connectionPrompt"].as_str().map(str::to_owned) else { return false };
      let a = app();
      let h = a.handle.clone();
      let _ = h.run_on_main_thread(move || {
        crate::os::clipboard_write(&prompt);
        let a = app();
        if let Some(w) = a.focused_window() {
          w.post(json!({ "type": "toast", "level": "info", "text": a.t("chatgpt.copied", &[]) }));
        }
      });
      true
    });
    let Some(w) = self.open(None, ChatWindowOpen { watch: Some(watch), ..Default::default() }) else { return };
    let Ok(msg) = serde_json::value::RawValue::from_string(r#"{"type":"connectChatgpt"}"#.into()) else { return };
    self.client().send(&w.view_id, msg);
  }

  pub fn refresh_menu(&self) {
    let h = self.handle.clone();
    let _ = h.run_on_main_thread(|| crate::menu::install(&app().handle));
  }

  pub fn theme_changed(&self) {
    let dark = self.dark();
    for w in self.windows() {
      w.set_theme(dark);
    }
    for b in self.browsers.lock().iter() {
      b.set_theme(dark);
    }
  }

  /// Quitting saves where every window was, then only disconnects: the engine keeps running turns and ends on its own
  /// once idle
  pub fn before_quit(&self) {
    if self.quitting.load(Ordering::SeqCst) {
      return;
    }
    self.save_windows();
    self.quitting.store(true, Ordering::SeqCst);
    let client = self.client().clone();
    let _ = tauri::async_runtime::block_on(async move { tokio::time::timeout(Duration::from_secs(2), client.dispose()).await });
    // A staged update goes in as the app quits (it starts the next time the app does); a download stops and resumes then
    if let Some(u) = self.updater() {
      u.stop();
      if u.staged().is_some()
        && let Err(e) = u.apply()
      {
        log(&format!("update: not installed at quit: {e}"));
      }
    }
  }
}

/// Clicking a system notification brings its session up
pub fn session_clicked(id: String) {
  let h = app().handle.clone();
  let _ = h.run_on_main_thread(move || app().show_session(&id));
}

/// The page never leaves the app: its own pages, and the loopback dev pages its browser frames show. Everything else
/// is refused (links reach the system browser through the engine's allowlist, `openExternal`)
fn allowed_navigation(url: &tauri::Url) -> bool {
  if acpira_desktop::page::is_app_url(url.as_str(), APP_ORIGIN) {
    return true;
  }
  match url.scheme() {
    "about" => true,
    "http" => matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")),
    _ => false,
  }
}

/// window.open of an http(s) page: the system browser
fn open_outside(url: &str) {
  if url.starts_with("http://") || url.starts_with("https://") {
    let _ = system::open_external(url);
  }
}

/// The system folder dialog, as a sheet on the window; pickedFolder carries no path when it was dismissed. Smoke runs
/// script the answers (ACPIRA_DESKTOP_PICK, path-list separated), across windows, without showing it
pub fn pick_folder(w: Arc<ChatWindow>) {
  static SCRIPTED: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
  let scripted = SCRIPTED.get_or_init(|| {
    let raw = std::env::var("ACPIRA_DESKTOP_PICK").unwrap_or_default();
    Mutex::new(std::env::split_paths(&raw).map(|p| p.to_string_lossy().into_owned()).filter(|p| !p.is_empty()).collect())
  });
  let next = {
    let mut s = scripted.lock();
    (!s.is_empty()).then(|| s.remove(0))
  };
  if let Some(path) = next {
    w.post(json!({ "type": "pickedFolder", "path": path }));
    return;
  }
  let window = w.window.clone();
  tauri::async_runtime::spawn(async move {
    let picked = rfd::AsyncFileDialog::new().set_parent(&window).set_can_create_directories(true).pick_folder().await;
    match picked {
      Some(dir) => w.post(json!({ "type": "pickedFolder", "path": dir.path().to_string_lossy() })),
      None => w.post(json!({ "type": "pickedFolder" })),
    }
  });
}

impl Platform for App {
  fn write_setting(&self, key: &str, value: Option<Value>) {
    self.settings.set(key, value);
  }

  fn setting(&self, key: &str) -> Option<Value> {
    App::setting(self, key)
  }

  fn log(&self, line: &str) {
    log(line);
  }

  fn watching(&self, session_id: &str) -> bool {
    self.windows().iter().any(|w| w.watching(session_id))
  }

  fn chime(&self) {
    if let Some(w) = self.focused_window() {
      w.post(json!({ "type": "chime" }));
    }
  }

  fn notification(&self, title: &str, body: &str, session_id: &str) {
    self.system_notification(title, body, Some(session_id));
  }

  fn toast(&self, level: &str, text: &str, open: Option<ToastOpen>) {
    App::toast(self, level, text, open);
  }

  fn open_window(&self, session_id: Option<String>) {
    let initial = Some(session_id.map(Initial::Session).unwrap_or(Initial::MostRecent));
    app().open(initial, ChatWindowOpen::default());
  }

  fn open_browser(&self, url: &str, folder: Option<&str>) -> Result<BrowserOpened, String> {
    app().open_browser(url, folder)
  }

  fn terminal(&self, r: TerminalRequest) {
    acpira_desktop::terminal::run_in_terminal(&r, &log);
  }

  fn open_in_editor(&self, editor: Editor, target: &FileTarget) -> bool {
    // The login shell's PATH finds an editor's command line; an app started from the Dock has almost none of its own
    let mut env: HashMap<String, String> = std::env::vars().collect();
    env.extend(self.login_env.lock().clone().unwrap_or_default());
    acpira_desktop::editors::open_in_editor(editor, target, &env, &self.paths.home, &log)
  }

  fn open_external(&self, url: &str) -> Result<(), String> {
    system::open_external(url)
  }

  fn reveal(&self, path: &str) -> Result<(), String> {
    system::reveal(path)
  }

  fn open_path(&self, path: &str) -> Result<(), String> {
    system::open_any(path)
  }

  fn t(&self, key: &str, params: &[(&str, &str)]) -> String {
    App::t(self, key, params)
  }
}

/// appearance.font for the page's inline config, when it is one of the fonts the bundle carries
pub fn page_font() -> Option<String> {
  let font = APP.get()?.setting("appearance.font")?;
  let font = font.as_str()?;
  acpira_shared::appearance::AXES.iter().find(|(k, _)| *k == "font").is_some_and(|(_, opts)| opts.contains(&font)).then(|| font.to_owned())
}

/// Where a new window's webview starts. WebKit starts on the empty `/blank` so the frame-rate switch is set before the
/// real page loads (the page is navigated to next); WebView2 has no such switch and would keep `/blank` as a back entry
/// (Alt+Left, the mouse's back button), so it starts on the page itself. The page's first command (acpira_attach) runs on
/// the main thread: a window opened there is registered before it can arrive, one opened elsewhere right after `build`
/// returns, well before the bundle has loaded
#[expect(clippy::expect_used, reason = "APP_ORIGIN is a constant absolute URL")]
fn first_url(host: &str) -> WebviewUrl {
  let url = if cfg!(target_os = "macos") { format!("{APP_ORIGIN}/blank").parse().expect("the blank URL parses") } else { page_url(host) };
  WebviewUrl::CustomProtocol(url)
}
