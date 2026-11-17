//! Settings page requests that need more than one call into the engine: shared config, plugins and an agent's controls

use std::sync::Arc;

use acpira_shared::protocol::HostMsg;
use acpira_shared::plugins::PluginAction;
use acpira_shared::shared_config::SharedAction;
use acpira_shared::transcript::AgentId;

use super::{BridgeCore, spawn_on};

impl BridgeCore {
  pub(super) async fn on_shared_action(self: &Arc<Self>, action: SharedAction) -> anyhow::Result<()> {
    let platform = &self.platform;
    let error = match self.settings.shared_action(action, &self.folder()).await {
      Ok(outcome) => {
        // A created file opens in the editor, a folder is revealed in the OS
        if let Some(p) = outcome.open {
          let path = p.to_string_lossy().into_owned();
          let r = if p.is_dir() { platform.reveal_in_os(&path).await } else { platform.open_resolved_file(&path, None).await };
          if let Err(e) = r {
            platform.log(&format!("shared config: opening {path} failed: {e}"));
          }
        }
        None
      }
      Err(e) => Some(format!("{e:#}")),
    };
    let view = self.settings.shared_view(&self.folder()).await;
    self.post_now(HostMsg::Shared { view, error });
    Ok(())
  }

  /// The plugin page (plugins/host.rs): every change, another engine's included, sends the view again
  pub(super) async fn on_plugins(self: &Arc<Self>) -> anyhow::Result<()> {
    {
      let mut sub = self.plugins_sub.lock();
      if sub.is_none() {
        let weak = self.me.clone();
        *sub = Some(self.plugins.subscribe(Arc::new(move || {
          if let Some(c) = weak.upgrade() {
            spawn_on(c.wire, async move {
              let view = c.plugins_view().await;
              c.post_now(HostMsg::Plugins { view, error: None });
            });
          }
        })));
      }
    }
    let view = self.plugins_view().await;
    self.post_now(HostMsg::Plugins { view, error: None });
    Ok(())
  }

  pub(super) async fn on_plugin_action(self: &Arc<Self>, action: PluginAction) -> anyhow::Result<()> {
    let platform = &self.platform;
    let error = match self.plugins.action(action).await {
      Ok(Some(dir)) => {
        if let Err(e) = platform.reveal_in_os(&dir.to_string_lossy()).await {
          platform.log(&format!("plugins: revealing {} failed: {e}", dir.display()));
        }
        None
      }
      Ok(None) => None,
      Err(e) => Some(format!("{e:#}")),
    };
    let view = self.plugins_view().await;
    self.post_now(HostMsg::Plugins { view, error });
    Ok(())
  }

  pub(super) async fn on_controls(self: &Arc<Self>, agent: AgentId, fresh: Option<bool>, cwd: Option<String>) -> anyhow::Result<()> {
    let manager = &self.manager;
    // A new session in a folder on an SSH host may start in the host's engine: then the models are the host CLI's
    if let Some(c) = match cwd.as_deref() {
      Some(folder) => manager.host_controls(&agent, folder, fresh == Some(true)).await,
      None => None,
    } {
      self.post_now(HostMsg::Controls { agent, controls: c.controls, shapes: c.shapes, commands: c.commands, cwd });
    } else if fresh != Some(true) {
      let controls = manager.known_controls(&agent).await;
      let shapes = manager.model_shapes(&agent);
      let commands = manager.known_commands(&agent).await;
      self.post_now(HostMsg::Controls { agent, controls, shapes, commands, cwd });
    } else {
      // The probe's initialize brings the version back too; a fresh inventory keeps the facts card in step
      let controls = manager.probe_controls(&agent).await;
      let shapes = manager.model_shapes(&agent);
      let commands = manager.known_commands(&agent).await;
      self.post_now(HostMsg::Controls { agent: agent.clone(), controls, shapes, commands, cwd });
      let inventory = self.settings.inventory(&agent, &self.folder()).await;
      self.post_now(HostMsg::Inventory { agent, inventory });
    }
    Ok(())
  }

  async fn plugins_view(&self) -> acpira_shared::plugins::PluginsView {
    let plugins = self.plugins.clone();
    tokio::task::spawn_blocking(move || plugins.view()).await.unwrap_or_default()
  }
}
