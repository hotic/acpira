//! A page's `ready`: its init, then the theme and the plugins' interface pieces it is subscribed to

use std::sync::Arc;

use acpira_shared::protocol::{HostMsg, InitState};
use acpira_shared::session_patch::Delivery;
use acpira_shared::session_window::VIEW_WINDOW_BYTES;

use super::{BridgeCore, HOST_VIEW_WAIT, spawn_on};

impl BridgeCore {
  pub(super) async fn on_ready(self: &Arc<Self>, patches: bool, list_patches: bool, window: bool) -> anyhow::Result<()> {
    let (manager, platform) = (&self.manager, &self.platform);
    self.patches.store(patches, std::sync::atomic::Ordering::Release);
    // A window moves with the patches that follow it: without them every push would open it again
    self.window.store(window && patches, std::sync::atomic::Ordering::Release);
    self.list_patches.store(list_patches, std::sync::atomic::Ordering::Release);
    // A view (re)opening is a cheap moment to re-check executables and pick up sessions another window created
    let m2 = manager.clone();
    spawn_on(self.wire, async move { m2.reprobe().await });
    manager.refresh_index().await;
    // Init waits for the session on screen to have a view, not for its agent: a stored session is shown as soon as
    // its record is read, while starting the agent and resuming can take seconds, and the page stays blank until init
    let ensure = {
      let (m, v) = (manager.clone(), self.viewer.clone());
      let preload = self.preload.lock().take();
      spawn_on(self.wire, async move {
        // The load begun at attach goes first: a session it could not open is replaced below, as without it
        if let Some(load) = preload {
          let _ = load.await;
        }
        m.ensure_active_for(&v).await
      })
    };
    tokio::select! {
      _ = ensure => {}
      _ = self.view_shown.notified() => {}
    }
    // A host session (remote run) is opened by asking its host for a view, and `ensure` returns before that view comes
    // back: init without it reads as "no project" on a project shell (the desktop's empty sheet flashes), so it waits
    // for the view, bounded. Looped because `view_shown` may hold a stale wake-up from an earlier push
    if let Some(id) = self.viewer.active_id() {
      let until = tokio::time::Instant::now() + HOST_VIEW_WAIT;
      while manager.awaits_host_view(&id) && tokio::time::timeout_at(until, self.view_shown.notified()).await.is_ok() {}
    }
    // A session another engine runs: its live view, not the read-only record copy shown until the first push lands
    manager.first_view_settled(self.viewer.active_id().as_deref()).await;
    // A page's first pushes start only now: init below carries everything they would have, and opening a long session
    // during the awaits above used to send its whole view once before init and once inside it.
    // The sent views stay locked from here until init is out: a session push racing init (the agent starting, a status
    // change) waits in `deliver` and goes as a patch against init's view. Before, it found no base yet and went whole,
    // a second copy of a long transcript just before init
    let mut sent = self.sent.lock();
    self.ready.store(true, std::sync::atomic::Ordering::Release);
    let (whole, active_parts) = match manager.session_msg(self.viewer.active_id().as_deref()) {
      Some(HostMsg::Session { session, parts, .. }) => (Some(session), parts),
      _ => (None, None),
    };
    // Put together only when the page gets it whole: a window is cut from the parts
    let mut active = None;
    // The page takes init's view as a whole and starts its cache over, keeping that view as the one on screen: it
    // becomes the base the next push of that session patches against, instead of a second copy of the transcript
    sent.clear();
    if let Some(parts) = active_parts.as_ref().filter(|_| self.patches.load(std::sync::atomic::Ordering::Acquire)) {
      if !self.window.load(std::sync::atomic::Ordering::Acquire) {
        sent.deliver(Some(parts));
      } else if let Delivery::Window(view) = sent.deliver_window(parts, VIEW_WINDOW_BYTES) {
        active = Some(view);
      }
    }
    let active = active.or_else(|| whole.map(|w| w.raw().clone()));
    let state = InitState {
      host: self.host,
      appearance: (self.appearance)(),
      agents: manager.agents(),
      accounts: manager.accounts(),
      account_actions: Some(manager.account_actions()),
      agent_installs: Some(manager.agent_installs()),
      hidden: manager.hidden(),
      sessions: self.viewer.listed_sessions(manager.sessions()),
      categories: manager.categories(),
      active,
      settings: self.settings.view_for(self.wire),
      locale: self.settings.locale_for(self.wire),
      home: platform.home(),
      // A project shell without a project yet has the home directory here, like a shell without a workspace
      cwd: manager.folder_for(Some(&self.viewer), None).unwrap_or_else(|| platform.home()),
      blob_base: self.blob_base.clone(),
      chats: Some(manager.chat_root().to_string_lossy().into_owned()),
    };
    // Init's list is the base of the next list patch, so the corrective list below goes as a patch (none if equal)
    *self.listed.lock() = list_patches.then(|| (0, state.sessions.clone()));
    self.emit(HostMsg::Init { state: Box::new(state) });
    drop(sent);
    let refused = std::mem::take(&mut *self.refused.lock());
    refused.into_iter().for_each(|m| self.queue(m));
    // A batched list may reach the page around init and lose to init's copy, and the manager does not repeat an
    // unchanged list: the batch gets the current one, ordered against every other list push
    manager.sessions_in_order(&self.viewer);
    // A reloaded page lost its theme layer: it goes again after every init (a task of its own: `sent`'s guard above
    // keeps this future from crossing an await)
    *self.theme.lock() = None;
    *self.ui.lock() = None;
    let me = self.clone();
    spawn_on(self.wire, async move {
      me.push_theme().await;
      me.push_ui().await;
    });
    let mut sub = self.theme_sub.lock();
    if sub.is_none() {
      let weak = self.me.clone();
      *sub = Some(self.plugins.subscribe(Arc::new(move || {
        if let Some(c) = weak.upgrade() {
          spawn_on(c.wire, async move {
            c.push_theme().await;
            c.push_ui().await;
          });
        }
      })));
    }
    drop(sub);
    let mut pages = self.pages_sub.lock();
    if pages.is_none() {
      let weak = self.me.clone();
      *pages = Some(self.plugins.runtimes().subscribe_pages(Arc::new(move |msg| {
        if let Some(c) = weak.upgrade() {
          c.post_now(msg);
        }
      })));
    }
    Ok(())
  }

  pub fn push_appearance(&self) {
    self.post_now(HostMsg::Appearance { appearance: (self.appearance)() });
  }

  /// The plugin page's view, built off the runtime threads (it reads and hashes the source folders)
  /// Send the theme layer when it differs from what the page has (nothing for a page that never had one and gets none)
  async fn push_theme(&self) {
    let plugins = self.plugins.clone();
    let Ok((css, skin, generation)) = tokio::task::spawn_blocking(move || plugins.theme()).await else { return };
    {
      // Compared without the generation: a commit that left the look alone sends nothing
      let key = format!("{css}\u{0}{}", skin.as_ref().map(|s| format!("{}/{}", s.plugin, s.snapshot)).unwrap_or_default());
      let mut last = self.theme.lock();
      if last.as_deref() == Some(key.as_str()) || (last.is_none() && css.is_empty() && skin.is_none()) {
        return;
      }
      *last = Some(key);
    }
    self.post_now(HostMsg::PluginTheme { css, skin, generation });
  }

  /// Send the interface pieces when they differ from what the page has, like the theme layer
  async fn push_ui(&self) {
    let plugins = self.plugins.clone();
    let Ok(slots) = tokio::task::spawn_blocking(move || plugins.ui_slots()).await else { return };
    {
      let mut last = self.ui.lock();
      if last.as_ref() == Some(&slots) || (last.is_none() && slots.is_empty()) {
        return;
      }
      *last = Some(slots.clone());
    }
    self.post_now(HostMsg::PluginUi { slots });
  }
}
