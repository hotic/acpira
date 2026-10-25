//! The menu bar as data: the extension's editor-free commands plus the page's own shortcuts, labelled in the app's
//! language. On macOS a menu item's key equivalent takes the key before the page sees it, so ⌘N / ⌘K / ⌘O / ⌘, arrive
//! here and go to the page as shellCommand, which runs what its own shortcut would. The standard items carry labels in the
//! app's language too: macOS adds items of its own (tiling, Quit and Keep Windows) in the system's language, and default
//! English labels stood out among them. The shell turns this tree into its toolkit's menu

use acpira_shared::settings::DESKTOP_LAYOUTS;

/// Commands the shell handles itself (the menu item's id)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
  NewSession,
  NewWindow,
  OpenFolder,
  Search,
  Settings,
  Browser,
  ToolPanel,
  /// View › Layout, by its DESKTOP_LAYOUTS value
  Layout(&'static str),
  ConnectChatgpt,
  ShowLog,
  Help,
  Reload,
  DevTools,
  ActualSize,
  ZoomIn,
  ZoomOut,
  /// Edit items the toolkit has no stock item for: the responder chain's own selectors
  PasteAndMatchStyle,
  Delete,
  StartSpeaking,
  StopSpeaking,
  /// The app menu's update item: check now, or restart into the staged version
  CheckUpdates,
  RestartToUpdate,
}

/// What the update item offers (None: the build has no updater, the item is left out)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateItem<'a> {
  Check,
  /// A version is staged: the item restarts into it
  Restart(&'a str),
}

impl Command {
  /// A stable id for the toolkit's menu events
  pub fn id(self) -> String {
    match self {
      Command::Layout(l) => format!("layout.{l}"),
      other => format!("{other:?}"),
    }
  }

  pub fn from_id(id: &str) -> Option<Command> {
    if let Some(l) = id.strip_prefix("layout.") {
      return DESKTOP_LAYOUTS.iter().find(|x| **x == l).map(|l| Command::Layout(l));
    }
    ALL.iter().copied().find(|c| c.id() == id)
  }
}

const ALL: [Command; 21] = [
  Command::NewSession,
  Command::NewWindow,
  Command::OpenFolder,
  Command::Search,
  Command::Settings,
  Command::Browser,
  Command::ToolPanel,
  Command::ConnectChatgpt,
  Command::ShowLog,
  Command::Help,
  Command::Reload,
  Command::DevTools,
  Command::ActualSize,
  Command::ZoomIn,
  Command::ZoomOut,
  Command::PasteAndMatchStyle,
  Command::Delete,
  Command::StartSpeaking,
  Command::StopSpeaking,
  Command::CheckUpdates,
  Command::RestartToUpdate,
];

/// Items the toolkit provides with the system's behaviour
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stock {
  About,
  Services,
  Hide,
  HideOthers,
  ShowAll,
  Quit,
  CloseWindow,
  Undo,
  Redo,
  Cut,
  Copy,
  Paste,
  SelectAll,
  Minimize,
  Zoom,
  Front,
  FullScreen,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
  Item { command: Command, label: String, accelerator: Option<&'static str> },
  Radio { command: Command, label: String, checked: bool },
  Stock { stock: Stock, label: String },
  Separator,
  Submenu { label: String, role: Option<SubmenuRole>, items: Vec<Entry> },
}

/// A submenu macOS manages: the Window menu lists the open windows after its items, Help gets the search field
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmenuRole {
  Window,
  Help,
}

pub type Translate<'a> = &'a dyn Fn(&str, &[(&str, &str)]) -> String;

/// `layout`: the window layout the page shows (acpira.desktopLayout), ticked in View › Layout; the menu is built again
/// when it changes, since ⌥⌘B folds the session column instead of the tool panel in the files layout. `update`: the
/// updater's item under About (in Help elsewhere), rebuilt when a version is staged
pub fn build_menu(t: Translate, app: &str, layout: &str, mac: bool, update: Option<UpdateItem>) -> Vec<Entry> {
  let named = [("app", app)];
  let item = |command, key: &str, accelerator| Entry::Item { command, label: t(key, &[]), accelerator };
  let stock = |stock, key: &str| Entry::Stock { stock, label: t(key, &named) };
  let sep = || Entry::Separator;
  let settings = || item(Command::Settings, "session.settings", Some("CmdOrCtrl+,"));
  let quit = || stock(Stock::Quit, "desktop.menu.quit");
  let update_item = update.map(|u| match u {
    UpdateItem::Check => item(Command::CheckUpdates, "desktop.menu.checkUpdates", None),
    UpdateItem::Restart(v) => Entry::Item { command: Command::RestartToUpdate, label: t("desktop.menu.restartToUpdate", &[("version", v)]), accelerator: None },
  });
  let mut bar = vec![];
  if mac {
    let mut items = vec![stock(Stock::About, "desktop.menu.about")];
    items.extend(update_item.clone());
    items.extend([sep(), settings()]);
    bar.push(Entry::Submenu {
      label: app.to_owned(),
      role: None,
      items: [items, vec![
        sep(),
        stock(Stock::Services, "desktop.menu.services"),
        sep(),
        stock(Stock::Hide, "desktop.menu.hide"),
        stock(Stock::HideOthers, "desktop.menu.hideOthers"),
        stock(Stock::ShowAll, "desktop.menu.showAll"),
        sep(),
        quit(),
      ]]
      .concat(),
    });
  }
  let mut file = vec![
    item(Command::NewSession, "session.new", Some("CmdOrCtrl+N")),
    item(Command::NewWindow, "desktop.menu.newWindow", Some("CmdOrCtrl+Shift+N")),
    item(Command::OpenFolder, "desktop.empty.open", Some("CmdOrCtrl+O")),
    sep(),
    item(Command::ConnectChatgpt, "desktop.menu.connectChatgpt", None),
    sep(),
    stock(Stock::CloseWindow, "desktop.menu.closeWindow"),
  ];
  if !mac {
    file.extend([sep(), settings(), sep(), quit()]);
  }
  bar.push(Entry::Submenu { label: t("desktop.menu.file", &[]), role: None, items: file });
  // Without the Edit items macOS sends no ⌘C / ⌘V / ⌘Z to the page
  let mut edit = vec![
    stock(Stock::Undo, "desktop.menu.undo"),
    stock(Stock::Redo, "desktop.menu.redo"),
    sep(),
    stock(Stock::Cut, "desktop.menu.cut"),
    stock(Stock::Copy, "desktop.menu.copy"),
    stock(Stock::Paste, "desktop.menu.paste"),
  ];
  if mac {
    edit.push(item(Command::PasteAndMatchStyle, "desktop.menu.pasteAndMatchStyle", Some("Alt+Shift+CmdOrCtrl+V")));
  }
  edit.extend([item(Command::Delete, "desktop.menu.delete", None), stock(Stock::SelectAll, "desktop.menu.selectAll")]);
  if mac {
    edit.extend([
      sep(),
      Entry::Submenu {
        label: t("desktop.menu.speech", &[]),
        role: None,
        items: vec![
          item(Command::StartSpeaking, "desktop.menu.startSpeaking", None),
          item(Command::StopSpeaking, "desktop.menu.stopSpeaking", None),
        ],
      },
    ]);
  }
  bar.push(Entry::Submenu { label: t("desktop.menu.edit", &[]), role: None, items: edit });
  let layouts = DESKTOP_LAYOUTS
    .iter()
    .map(|l| Entry::Radio { command: Command::Layout(l), label: t(&format!("settings.desktopLayout.{l}"), &[]), checked: *l == layout })
    .collect();
  bar.push(Entry::Submenu {
    label: t("desktop.menu.view", &[]),
    role: None,
    items: vec![
      item(Command::Search, "session.search", Some("CmdOrCtrl+K")),
      item(Command::ToolPanel, if layout == "files" { "desktop.sessionList" } else { "panel.label" }, Some("Alt+CmdOrCtrl+B")),
      item(Command::Browser, "desktop.menu.browser", None),
      sep(),
      Entry::Submenu { label: t("settings.desktopLayout", &[]), role: None, items: layouts },
      sep(),
      item(Command::Reload, "desktop.menu.reload", Some("CmdOrCtrl+R")),
      item(Command::DevTools, "desktop.menu.devTools", Some("Alt+CmdOrCtrl+I")),
      sep(),
      item(Command::ActualSize, "desktop.menu.actualSize", Some("CmdOrCtrl+0")),
      item(Command::ZoomIn, "desktop.menu.zoomIn", Some("CmdOrCtrl+=")),
      item(Command::ZoomOut, "desktop.menu.zoomOut", Some("CmdOrCtrl+-")),
      sep(),
      stock(Stock::FullScreen, "desktop.menu.fullScreen"),
    ],
  });
  let window = if mac {
    vec![
      stock(Stock::Minimize, "desktop.menu.minimize"),
      stock(Stock::Zoom, "desktop.menu.zoom"),
      sep(),
      stock(Stock::Front, "desktop.menu.front"),
    ]
  } else {
    vec![stock(Stock::Minimize, "desktop.menu.minimize"), stock(Stock::CloseWindow, "desktop.menu.closeWindow")]
  };
  bar.push(Entry::Submenu { label: t("desktop.menu.window", &[]), role: Some(SubmenuRole::Window), items: window });
  let mut help = vec![item(Command::Help, "desktop.help", None), item(Command::ShowLog, "desktop.menu.showLog", None)];
  if !mac && let Some(u) = update_item {
    help.extend([sep(), u]);
  }
  bar.push(Entry::Submenu { label: t("desktop.menu.help", &[]), role: Some(SubmenuRole::Help), items: help });
  bar
}

/// The page command a menu command becomes (ShellCommand in packages/shared/protocol.ts), when it is one
pub fn shell_command(c: Command) -> Option<&'static str> {
  Some(match c {
    Command::NewSession => "newSession",
    Command::OpenFolder => "openFolder",
    Command::Search => "search",
    Command::Settings => "settings",
    Command::Browser => "browser",
    Command::ToolPanel => "toolPanel",
    Command::Layout("files") => "layoutFiles",
    Command::Layout(_) => "layoutChat",
    _ => return None,
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use acpira_shared::i18n::{Locale, translate};

  fn labels(entries: &[Entry], out: &mut Vec<String>) {
    for e in entries {
      match e {
        Entry::Item { label, .. } | Entry::Radio { label, .. } | Entry::Stock { label, .. } => out.push(label.clone()),
        Entry::Submenu { label, items, .. } => {
          out.push(label.clone());
          labels(items, out);
        }
        Entry::Separator => {}
      }
    }
  }

  #[test]
  fn every_label_is_translated_in_all_nine_languages() {
    for locale in [Locale::ZhCn, Locale::ZhTw, Locale::En, Locale::Ja, Locale::Ko, Locale::Es, Locale::De, Locale::Fr, Locale::Ru] {
      let t = |k: &str, p: &[(&str, &str)]| translate(locale, k, p);
      let mut all = vec![];
      labels(&build_menu(&t, "Acpira", "chat", true, Some(UpdateItem::Check)), &mut all);
      labels(&build_menu(&t, "Acpira", "chat", true, Some(UpdateItem::Restart("1.11.0"))), &mut all);
      for l in &all {
        assert!(
          !l.is_empty() && !l.starts_with("desktop.") && !l.starts_with("settings.") && !l.starts_with("session.") && !l.contains("{app}")
            && !l.contains("{version}"),
          "{locale:?}: {l}"
        );
      }
    }
  }

  #[test]
  fn the_layout_is_ticked_and_the_panel_item_follows_it() {
    let t = |k: &str, _: &[(&str, &str)]| k.to_owned();
    let view = |layout| {
      let bar = build_menu(&t, "Acpira", layout, true, None);
      let Some(Entry::Submenu { items, .. }) =
        bar.into_iter().find(|e| matches!(e, Entry::Submenu { label, .. } if label == "desktop.menu.view"))
      else {
        panic!()
      };
      items
    };
    let files = view("files");
    assert!(matches!(&files[1], Entry::Item { label, .. } if label == "desktop.sessionList"));
    let Entry::Submenu { items, .. } = &files[4] else { panic!() };
    assert_eq!(
      items,
      &[
        Entry::Radio { command: Command::Layout("chat"), label: "settings.desktopLayout.chat".into(), checked: false },
        Entry::Radio { command: Command::Layout("files"), label: "settings.desktopLayout.files".into(), checked: true },
      ]
    );
    assert!(matches!(&view("chat")[1], Entry::Item { label, .. } if label == "panel.label"));
  }

  #[test]
  fn ids_round_trip_and_page_commands_map() {
    for c in ALL.iter().copied().chain([Command::Layout("chat"), Command::Layout("files")]) {
      assert_eq!(Command::from_id(&c.id()), Some(c));
    }
    assert_eq!(Command::from_id("layout.nope"), None);
    assert_eq!(shell_command(Command::Layout("files")), Some("layoutFiles"));
    assert_eq!(shell_command(Command::NewWindow), None);
  }

  #[test]
  fn the_update_item_sits_under_about_and_names_a_staged_version() {
    let t = |k: &str, p: &[(&str, &str)]| p.iter().fold(k.to_owned(), |acc, (n, v)| format!("{acc}:{n}={v}"));
    let app_menu = |u| {
      let Some(Entry::Submenu { items, .. }) = build_menu(&t, "Acpira", "chat", true, u).into_iter().next() else { panic!() };
      items
    };
    assert!(matches!(&app_menu(Some(UpdateItem::Check))[1], Entry::Item { command: Command::CheckUpdates, .. }));
    assert!(
      matches!(&app_menu(Some(UpdateItem::Restart("1.11.0")))[1], Entry::Item { command: Command::RestartToUpdate, label, .. } if label.ends_with("version=1.11.0"))
    );
    assert!(matches!(&app_menu(None)[1], Entry::Separator));
  }

  #[test]
  fn other_platforms_keep_settings_and_quit_in_the_file_menu() {
    let t = |k: &str, _: &[(&str, &str)]| k.to_owned();
    let bar = build_menu(&t, "Acpira", "chat", false, None);
    let Entry::Submenu { label, items, .. } = &bar[0] else { panic!() };
    assert_eq!(label, "desktop.menu.file");
    assert!(items.iter().any(|e| matches!(e, Entry::Stock { stock: Stock::Quit, .. })));
  }
}
