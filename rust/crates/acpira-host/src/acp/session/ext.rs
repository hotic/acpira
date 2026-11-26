//! Narrow session-facing seams for plugin runtimes, plugin activation cards, and host MCP.
//! Implemented by `plugins` (L10) and `host_mcp` (L11) so `acp::session` (L08) stays strictly downward-only.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use acpira_shared::transcript::{PluginCard, PluginCardUndo};

use crate::acp::session::AcpSession;
use crate::acp::session::plugin_hooks::RedirectFn;
use crate::acp::transport::rpc::BoxFuture;

/// `contributes.commands[]`: `/name` in the composer of every session
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandDef {
  pub name: String,
  #[serde(default)]
  pub description: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub hint: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub prompt: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub button: Option<String>,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub draft: bool,
  /// The button's glyph, one of `BUTTON_ICONS`; absent: a bolt, or a wand for a `draft` command
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub icon: Option<String>,
  /// The button's look, one of `BUTTON_LOOKS`; absent: `brand`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub look: Option<String>,
  /// Whether the button takes the draft as its arguments: `draft` or `none`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub input: Option<String>,
  /// The button opens a menu of these instead of running the command
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub menu: Vec<MenuItemDef>,
}

/// One row of a command button's menu (`MENU_KINDS`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuItemDef {
  #[serde(default)]
  pub id: String,
  pub kind: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub group: Option<String>,
  #[serde(default)]
  pub label: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  /// The group's choice, or a toggle that starts on, until the user picks otherwise
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub default: bool,
}

impl CommandDef {
  /// `input` as it applies
  pub fn takes_draft(&self) -> bool {
    match self.input.as_deref() {
      Some(i) => i == "draft",
      None => self.draft || self.prompt.as_deref().is_some_and(|p| p.contains("$ARGUMENTS")),
    }
  }

  /// The menu's state in a session: each group's choice and each toggle, defaults filled in
  pub fn menu_picks(&self, stored: Option<&Map<String, Value>>) -> Map<String, Value> {
    let mut out = Map::new();
    for item in &self.menu {
      match (item.kind.as_str(), item.group.as_deref()) {
        ("radio", Some(group)) => {
          let kept = stored
            .and_then(|s| s.get(group))
            .and_then(Value::as_str)
            .filter(|id| self.menu.iter().any(|i| i.kind == "radio" && i.group.as_deref() == Some(group) && i.id == *id));
          match kept {
            Some(id) => {
              out.insert(group.to_owned(), Value::from(id));
            }
            None if item.default || !out.contains_key(group) => {
              out.insert(group.to_owned(), Value::from(item.id.as_str()));
            }
            None => {}
          }
        }
        ("toggle", _) => {
          let on = stored.and_then(|s| s.get(&item.id)).and_then(Value::as_bool).unwrap_or(item.default);
          out.insert(item.id.clone(), Value::from(on));
        }
        _ => {}
      }
    }
    out
  }

  /// The picks after the user chose `id`: a radio becomes its group's choice, a toggle flips
  pub fn menu_pick(&self, current: &Map<String, Value>, id: &str) -> Option<Map<String, Value>> {
    let item = self.menu.iter().find(|i| i.id == id && i.kind != "separator")?;
    let mut next = current.clone();
    match (item.kind.as_str(), item.group.as_deref()) {
      ("radio", Some(group)) => next.insert(group.to_owned(), Value::from(id)),
      ("toggle", _) => next.insert(id.to_owned(), Value::from(!current.get(id).and_then(Value::as_bool).unwrap_or(item.default))),
      _ => return None,
    };
    Some(next)
  }
}

/// One slash command contributed by an active plugin
#[derive(Debug, Clone, PartialEq)]
pub struct PluginCommand {
  pub plugin: String,
  pub def: CommandDef,
}

/// Who made a plugin state change: the user on a page, a session's agent through its card, or the command line
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
  /// `user`, `session` or `cli`
  pub by: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub session: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub agent: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<String>,
}

impl Provenance {
  pub fn user() -> Provenance {
    Provenance { by: "user".into(), ..Default::default() }
  }

  pub fn cli() -> Provenance {
    Provenance { by: "cli".into(), ..Default::default() }
  }
}

/// Outcome of freezing and checking a plugin before showing an activation card
pub enum PluginOfferOutcome {
  Error(String),
  AlreadyActive(String),
  Offer(Box<PluginCard>),
}

/// Narrow plugin host surface for offering, applying, and undoing plugin activation cards
pub trait PluginCardPort: Send + Sync {
  fn prepare_offer(self: Arc<Self>, id: String, card_id: String) -> BoxFuture<PluginOfferOutcome>;
  fn offered(&self);
  fn apply_card<'a>(&'a self, card: &'a PluginCard, provenance: Provenance) -> Pin<Box<dyn Future<Output = Result<Option<PluginCardUndo>>> + Send + 'a>>;
  fn undo_card<'a>(&'a self, card: &'a PluginCard, provenance: Provenance) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
}

/// Step callback passed into `PluginRuntimePort::chain`
pub type ChainStep<'a> = &'a mut (dyn FnMut(&str, Value, &mut Value) -> bool + Send);

/// Future returned by plugin hook dispatch methods
pub type HookOutcomeFuture<'a> = Pin<Box<dyn Future<Output = Option<Result<Value, String>>> + Send + 'a>>;

/// Narrow plugin runtime surface consumed by `acp::session`
pub trait PluginRuntimePort: Send + Sync {
  fn has_hook(&self, hook: &str) -> bool;
  fn declared(&self) -> Vec<String>;
  fn hears(&self, event: &str) -> bool;
  fn emit_value(&self, event: &str, data: Value);
  fn contributed_commands(&self) -> Vec<PluginCommand>;
  fn contributed_skills(&self) -> Vec<PathBuf>;
  fn shipped_skills(&self) -> Option<PathBuf>;
  fn hook_of<'a>(&'a self, plugin: &'a str, hook: &'a str, payload: Value) -> HookOutcomeFuture<'a>;
  fn hook_clicked<'a>(&'a self, plugin: &'a str, hook: &'a str, payload: Value) -> HookOutcomeFuture<'a>;
  fn chain<'a>(&'a self, hook: &'a str, payload: Value, step: ChainStep<'a>) -> Pin<Box<dyn Future<Output = Value> + Send + 'a>>;
  fn redirect(&self) -> Option<RedirectFn>;
}

/// Narrow host MCP surface consumed by `acp::session`
pub trait SessionHostMcp: Send + Sync {
  fn entry_for(&self, agent: &str) -> Option<Value>;
  fn entry_for_session(&self, agent: &str, session: &Arc<AcpSession>, thread: Option<&str>, depth: u32) -> Option<Value>;
  fn host_stdio_entry(&self, session: &Arc<AcpSession>, name: &str) -> Option<Value>;
  fn refuse(&self, agent: &str);
  fn refused(&self, agent: &str) -> bool;
  fn plugin_cards(&self) -> Option<Arc<dyn PluginCardPort>>;
}
