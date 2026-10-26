//! The Acpira desktop app in a Tauri shell (docs/dev/desktop.md): WKWebView (macOS), WebView2 (Windows) or WebKitGTK
//! (Linux) windows running the webview bundle on the app protocol (acpira_desktop::page::APP_ORIGIN), the persistent
//! engine behind them, and the system side of the IDE actions. The logic outside the windows lives in
//! rust/crates/acpira-desktop, which the rust/ workspace's gates test
// A GUI program on Windows: no console window comes up with it
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod browser;
mod chat_window;
mod commands;
mod embedded;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod mac;
mod menu;
mod os;
mod page;
mod serve;
mod smoke;
#[cfg(windows)]
mod win;

use acpira_desktop::single_instance::{Instance, claim, links_in};
use tauri::{Manager, RunEvent, WindowEvent};

/// The page transport (src/init.js), injected before the page's own scripts, with the app protocol's origin filled in
pub static INIT_SCRIPT: std::sync::LazyLock<String> =
  std::sync::LazyLock::new(|| include_str!("init.js").replace("__ACPIRA_APP_ORIGIN__", acpira_desktop::page::APP_ORIGIN));

fn main() {
  let args: Vec<String> = std::env::args().collect();
  // Windows ships one exe: its first argument picks the engine's roles (`serve`, `agent`, `stdio`, …), anything else (no
  // argument, a session link) is the app's window
  #[cfg(windows)]
  if acpira_host::cli::is_engine_command(args.get(1).map(String::as_str)) {
    win::attach_console(args.get(1).map(String::as_str));
    std::process::exit(acpira_host::cli::run(args[1..].to_vec()));
  }
  // A panic aborts (Cargo.toml's release profile): its message goes to the app's log first, since a window process's
  // stderr reaches no one on Windows or when started from the Dock
  let default_hook = std::panic::take_hook();
  std::panic::set_hook(Box::new(move |info| {
    app::log(&format!("panic: {info}"));
    default_hook(info);
  }));
  // An exe the updater restarted (Windows) waits for the one it replaces to exit, which still holds the single instance
  // SAFETY: no other thread exists yet
  unsafe { acpira_desktop::update::swap::wait_for_predecessor() };
  let context = tauri::generate_context!();
  let paths = app::Paths::resolve(&context.config().identifier);
  // A second launch hands its arguments (a session link) to the running app and exits
  let primary = match claim(&paths.user_data, &args) {
    Ok(Instance::Primary(p)) => Some(p),
    Ok(Instance::Forwarded) => return,
    Err(e) => {
      eprintln!("single instance: {e}; running without it");
      None
    }
  };
  let links = links_in(&args);
  // Without a WebView2 Runtime no window can open: say so instead of failing silently
  #[cfg(windows)]
  if !win::webview2_available() {
    win::webview2_missing();
    return;
  }
  #[cfg(target_os = "macos")]
  let background = std::env::var("ACPIRA_DESKTOP_BACKGROUND").as_deref() == Ok("1");

  let built = tauri::Builder::default()
    .register_asynchronous_uri_scheme_protocol("acpira-app", |_ctx, request, responder| serve::handle(request, responder))
    .invoke_handler(tauri::generate_handler![
      commands::acpira_attach,
      commands::acpira_post,
      commands::acpira_window,
      commands::acpira_clipboard,
      commands::acpira_drag_paths,
      smoke::acpira_smoke_result
    ])
    .on_window_event(|window, event| {
      let Some(a) = app::try_app() else { return };
      let label = window.label();
      match event {
        WindowEvent::Focused(true) => {
          if let Some(w) = a.window_by_label(label) {
            a.on_focus(&w);
          }
        }
        WindowEvent::CloseRequested { .. } => a.window_closing(label),
        WindowEvent::Destroyed => a.window_closed(label),
        WindowEvent::Moved(_) => a.schedule_save(),
        WindowEvent::Resized(_) => {
          a.schedule_save();
          a.window_resized(label);
        }
        WindowEvent::ThemeChanged(_) => a.theme_changed(),
        _ => {}
      }
    })
    .on_menu_event(|h, e| menu::on_event(h, e.id().as_ref()))
    .setup(move |tauri_app| {
      // A background run (smoke) never activates the app: no Dock icon, no focus taken, windows behind everything
      #[cfg(target_os = "macos")]
      if background {
        tauri_app.set_activation_policy(tauri::ActivationPolicy::Accessory);
      }
      let handle = tauri_app.handle().clone();
      let a = app::App::init(handle.clone(), paths, links);
      // Linux: the colour scheme's changes, which reach no window as ThemeChanged there (linux::watch_theme)
      #[cfg(target_os = "linux")]
      linux::watch_theme(|| {
        if let Some(a) = app::try_app() {
          a.theme_changed();
        }
      });
      if let Some(p) = primary {
        let h = handle.clone();
        let listening = p.listen(move |args| {
          let _ = h.run_on_main_thread(move || app::app().second_instance(args));
        });
        if let Err(e) = listening {
          app::log(&format!("single instance: {e}"));
        }
        // Held for the life of the app: dropping it gives the next launch the lock
        tauri_app.manage(std::sync::Mutex::new(Some(p)));
      }
      menu::install(&handle);
      // Mouse-wheel notches eased like Chromium's (mac.rs smooth_wheel); a no-op elsewhere
      os::smooth_wheel(std::sync::Arc::new(app::log));
      a.restore();
      a.start_engine();
      smoke::start();
      Ok(())
    })
    .build(context);
  let tauri_app = match built {
    Ok(a) => a,
    Err(e) => {
      eprintln!("acpira desktop: {e}");
      std::process::exit(1);
    }
  };
  tauri_app.run(|_handle, event| {
    let Some(a) = app::try_app() else { return };
    match event {
      // The last window closed: macOS keeps the app (and its engine connection); turns go on, notifications still come
      RunEvent::ExitRequested { code: None, api, .. } if !a.quitting.load(std::sync::atomic::Ordering::SeqCst) => {
        if cfg!(target_os = "macos") {
          api.prevent_exit();
        } else {
          a.before_quit();
        }
      }
      RunEvent::ExitRequested { .. } | RunEvent::Exit => a.before_quit(),
      #[cfg(target_os = "macos")]
      RunEvent::Reopen { .. } => a.reopen(),
      #[cfg(target_os = "macos")]
      RunEvent::Opened { urls } => {
        for u in urls {
          a.take_link(u.as_str());
        }
      }
      _ => {}
    }
  });
}
