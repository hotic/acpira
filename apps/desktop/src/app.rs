//! The desktop app's main process: one connection to the persistent engine (`acpira serve`, its own socket key), any
//! number of chat windows (each an engine view), browser panel windows (no view; this side holds their preview leases),
//! and the system side of the IDE actions; see docs/dev/desktop.md

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use acpira_desktop::browser::{BrowserHistory, browser_look};
use acpira_desktop::client::SidecarClient;
use acpira_desktop::context_api::ContextSlots;
use acpira_desktop::page::APP_ORIGIN;
use acpira_desktop::settings::SettingsStore;
use acpira_desktop::shared_settings::SharedSettingsFile;
use acpira_desktop::update::{Phase, Updater};
use acpira_desktop::window_state::{Placement, WindowEntry, WindowStateStore, WindowsState};
use acpira_shared::i18n::{Language, Locale, resolve_locale, translate};
use parking_lot::Mutex;
use serde_json::{Value, json};
use tauri::{AppHandle, WebviewUrl};

use crate::browser::BrowserPanelWindow;
use crate::chat_window::{ChatWindow, Watch, page_url};

mod actions;
mod context;
mod engine;
mod links;
mod platform;
mod preview;
mod updates;
mod windows;

pub use self::links::session_clicked;
pub use self::platform::pick_folder;

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
    } else if let Some(res) = linux_resources(&exe_dir) {
      // A Linux package (deb, rpm, AppImage): the shell in <prefix>/bin, the engine in <prefix>/libexec/acpira, the page
      // and the picker in <prefix>/lib/Acpira (scripts/package-linux.mjs)
      let prefix = exe_dir.parent().unwrap_or(&exe_dir);
      (res.join("webview"), res.join("picker.js"), vec![prefix.join("libexec").join("acpira").join("acpira-engine")])
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

/// A Linux package's resources, Tauri's layout for deb, rpm and AppImage alike: <prefix>/lib/<productName> beside the
/// <prefix>/bin that holds this exe
fn linux_resources(exe_dir: &Path) -> Option<PathBuf> {
  if !cfg!(target_os = "linux") {
    return None;
  }
  let res = exe_dir.parent()?.join("lib").join("Acpira");
  res.join("webview").is_dir().then_some(res)
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
  /// Shell-local attention, not the engine's shared unread marks: a covered window can still show the session
  pub(crate) attention: Mutex<HashSet<String>>,
  updater: OnceLock<Arc<Updater>>,
  /// The updater's phase as the menu and the toast last saw it
  update_phase: Mutex<Phase>,
  /// The menu bar as the page draws it where there is no global one (Windows; menu.rs install), sent to each page on
  /// ready and on every change
  page_menu: Mutex<Option<Value>>,
  /// When each page's WebContent process last ended (page_process_ended), by window label
  page_ended: Mutex<HashMap<String, Instant>>,
  /// The data folder was missing at launch and the user has not said yes yet (acpira_desktop::first_run): no engine runs
  first_run: AtomicBool,
  /// A new desktop profile still needs its intro and tour, independently of the shared data folder
  welcome_pending: AtomicBool,
  /// The login shell's environment is in, so a yes starts the engine at once
  env_ready: AtomicBool,
  /// The first-run intro's agent row
  detection: Mutex<Detection>,
}

/// What the first-run probe found, or the windows waiting for it
enum Detection {
  Running(Vec<Arc<ChatWindow>>),
  Found(Value),
  /// Logged; the intro leaves its row empty rather than claim nothing was found
  Failed,
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
    let first_run = !paths.data_home.exists();
    let welcome_pending = settings.fresh;
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
      attention: Mutex::new(HashSet::new()),
      updater: OnceLock::new(),
      update_phase: Mutex::new(Phase::Idle),
      page_menu: Mutex::new(None),
      page_ended: Mutex::new(HashMap::new()),
      first_run: AtomicBool::new(first_run),
      welcome_pending: AtomicBool::new(welcome_pending),
      env_ready: AtomicBool::new(false),
      detection: Mutex::new(Detection::Running(vec![])),
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
