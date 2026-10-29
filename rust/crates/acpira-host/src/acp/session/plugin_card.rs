//! `plugin_activate` (`docs/dev/plugins.md`, Activation): an agent of this session asks to enable a plugin it wrote.
//! The plugin is frozen and checked statically first (nothing of it runs), then a plugin card joins the agent's turn
//! (`AgentTurn::plugin_cards`). The card stays clickable after the turn: applying it is the user's confirmation, and
//! undoing it puts back what it replaced. The call waits as long as its client allows; an apply after that still
//! enables the plugin, and the agent is told in a queued message when the plugin gives it something to use

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use acpira_shared::plugins::PluginCardOp;
use acpira_shared::transcript::{PluginCard, Turn};

use crate::acp::session::AcpSession;
use crate::acp::session::gates::{PendingQuestion, QuestionReply, Target};
use crate::i18n::tp;
use crate::plugins::host::PluginHost;
use crate::plugins::store::Provenance;
use crate::relay::wire::HubReply;

/// How long the call waits when the client named no bound (the 50 s every client but Claude gets, `host_mcp.rs`)
const DEFAULT_WAIT: Duration = Duration::from_secs(50);
const MAX_WAIT: Duration = Duration::from_secs(25 * 60);
const PROGRESS_EVERY: Duration = Duration::from_secs(5);

/// A `plugin_activate` call waiting on its card: what the agent is told once the card is applied, and the plugin host
/// the call came through (the one a click applies to when this session's own hub has none), kept after the answer
pub(crate) struct PluginWait {
  tx: Option<oneshot::Sender<HubReply>>,
  host: Arc<PluginHost>,
}

impl AcpSession {
  pub async fn plugin_activate(self: Arc<Self>, host: Arc<PluginHost>, id: String, wait_secs: Option<u64>, out: mpsc::UnboundedSender<HubReply>) {
    let prepared = {
      let (host, id) = (host.clone(), id.clone());
      tokio::task::spawn_blocking(move || host.prepare(&id)).await
    };
    let frozen = match prepared {
      Ok(Ok(f)) => f,
      Ok(Err(e)) => {
        let _ = out.send(HubReply::Error(format!("{e:#}")));
        return;
      }
      Err(e) => {
        let _ = out.send(HubReply::Error(format!("the check did not finish: {e}")));
        return;
      }
    };
    if !frozen.findings.ok() {
      let _ = out.send(HubReply::Error(format!(
        "The plugin was not offered to the user. Fix these in {} and call plugin_activate again:\n{}",
        host.store().source(&id).display(),
        frozen.findings.text()
      )));
      return;
    }
    // In effect already: enabled at this version, and the skin when it is a theme
    let loaded = host.loaded();
    let enabled = loaded.state.plugins.get(&id).is_some_and(|e| e.enabled && e.snapshot == frozen.hash);
    let skinned = frozen.manifest.contributes.theme.is_none() || loaded.skin().is_some_and(|a| a.id == id);
    if enabled && skinned {
      let _ = out.send(HubReply::Done(format!("This version of {id} is in effect already.")));
      return;
    }
    let card = host.card(&frozen, format!("plugin-{}", crate::util::random_hex(6)));
    let card_id = card.id.clone();
    let (tx, mut rx) = oneshot::channel();
    {
      let mut c = self.core.lock();
      let Some(turn) = c.state.turns.iter_mut().rev().find_map(Turn::as_agent_mut) else {
        drop(c);
        let _ = out.send(HubReply::Error("There is no turn of yours to show the card in.".into()));
        return;
      };
      turn.plugin_cards.push(card);
      self.plugin_waits.lock().insert(card_id.clone(), PluginWait { tx: Some(tx), host: host.clone() });
      self.touch(&mut c);
    }
    let deadline = wait_secs.map(Duration::from_secs).unwrap_or(DEFAULT_WAIT).min(MAX_WAIT);
    let until = tokio::time::Instant::now() + deadline;
    let mut tick = tokio::time::interval(PROGRESS_EVERY);
    tick.tick().await;
    let answer = loop {
      tokio::select! {
        r = &mut rx => break r.ok(),
        _ = tick.tick() => {
          if out.send(HubReply::Progress("Waiting for the user to apply the plugin card".into())).is_err() {
            break None;
          }
        }
        _ = tokio::time::sleep_until(until) => break None,
      }
    };
    if let Some(reply) = answer {
      let _ = out.send(reply);
      return;
    }
    let _ = out.send(HubReply::Done(format!(
      "The card offering {id} is in the conversation; the user applies it when they like, also after this turn. If it \
       gives you tools or skills you are told in a message then (tools appear in your list, in a new session for \
       clients that do not list tools again). Go on with other work meanwhile; do not call plugin_activate again for \
       the same version."
    )));
    // A later apply: the agent hears of it only when there is something for it to use. The wait holds the session
    // weakly, so a card nobody clicks never keeps a closed session alive (its sender goes with the session)
    let uses = !frozen.manifest.contributes.tools.is_empty() || frozen.manifest.contributes.skills.is_some() || !frozen.manifest.contributes.commands.is_empty();
    if !uses {
      return;
    }
    let me = Arc::downgrade(&self);
    drop(self);
    tokio::spawn(async move {
      let Ok(reply) = rx.await else { return };
      let Some(me) = me.upgrade() else { return };
      let text = match reply {
        HubReply::Done(t) | HubReply::Error(t) => t,
        _ => return,
      };
      me.enqueue(format!("[Acpira] {text}"), vec![], None).await;
    });
  }

  /// A card's button: apply the version it offers, or undo what applying it changed. A failure is a toast; the
  /// card's state follows the plugins' state either way
  pub async fn plugin_card(self: &Arc<Self>, card_id: &str, op: PluginCardOp) {
    let own = self.deps.host_mcp.as_ref().and_then(|m| m.hub.as_ref()).and_then(|h| h.plugin_host());
    let Some(host) = own.or_else(|| self.plugin_waits.lock().get(card_id).map(|w| w.host.clone())) else {
      self.notify("Plugins are not available in this host.");
      return;
    };
    let Some(card) = self.find_card(card_id) else { return };
    let provenance = self.plugin_provenance();
    match op {
      PluginCardOp::Apply => match host.apply_card(&card, provenance).await {
        Ok(undo) => {
          if let Some(undo) = undo {
            self.update_card(card_id, |c| c.undo = Some(undo));
          }
          let tools: Vec<String> = host
            .loaded()
            .active
            .iter()
            .filter(|a| a.id == card.plugin)
            .flat_map(|a| a.manifest.contributes.tools.iter().map(|t| crate::plugins::manifest::served_name(&card.plugin, &t.name)))
            .collect();
          let tools = if tools.is_empty() { String::new() } else { format!(" Its tools on Acpira's MCP server: {}.", tools.join(", ")) };
          let text = format!("The user applied {}.{tools} Clients that list tools again see them now; others in a new session.", card.plugin);
          if let Some(tx) = self.plugin_waits.lock().get_mut(card_id).and_then(|w| w.tx.take()) {
            let _ = tx.send(HubReply::Done(text));
          }
        }
        Err(e) => self.notify(&tp("host.plugin.applyFailed", &[("name", &card.name), ("error", &format!("{e:#}"))])),
      },
      PluginCardOp::Undo => match host.undo_card(&card, provenance).await {
        Ok(()) => self.update_card(card_id, |c| c.undo = None),
        Err(e) => self.notify(&tp("host.plugin.undoFailed", &[("name", &card.name), ("error", &format!("{e:#}"))])),
      },
    }
  }

  fn find_card(&self, card_id: &str) -> Option<PluginCard> {
    let c = self.core.lock();
    c.state.turns.iter().filter_map(Turn::as_agent).flat_map(|t| t.plugin_cards.iter()).find(|p| p.id == card_id).cloned()
  }

  fn update_card(&self, card_id: &str, f: impl FnOnce(&mut PluginCard)) {
    let mut c = self.core.lock();
    let Some(card) = c.state.turns.iter_mut().filter_map(Turn::as_agent_mut).flat_map(|t| t.plugin_cards.iter_mut()).find(|p| p.id == card_id) else { return };
    f(card);
    self.touch(&mut c);
  }

  /// This session's agent and model, for the record of who asked
  fn plugin_provenance(&self) -> Provenance {
    let c = self.core.lock();
    let model = acpira_shared::model_shapes::session_model(&c.state.controls).map(|m| m.id);
    Provenance { by: "session".into(), session: Some(self.id.clone()), agent: Some(self.agent.clone()), model }
  }
}

/// The longest a plugin's question may wait for the user
pub const ASK_MAX_WAIT: Duration = Duration::from_secs(25 * 60);

impl AcpSession {
  /// A plugin's own question (`ui/ask`): a card in this session naming the plugin, answered by the user. The answer as
  /// a question card gives it (`answers`, `skipped`, `cancelled`), or None when `wait` passed first (the card is withdrawn)
  pub async fn plugin_ask(self: &Arc<Self>, plugin: &str, questions: Vec<acpira_shared::transcript::Question>, message: Option<String>, wait: Duration) -> Option<Value> {
    let head = tp("host.plugin.asks", &[("name", plugin)]);
    let message = Some(match message.filter(|m| !m.trim().is_empty()) {
      Some(m) => format!("{head}\n\n{m}"),
      None => head,
    });
    let (rx, block_id) = {
      let mut c = self.core.lock();
      let block_id = self.open_question(&mut c, &Target::Root, questions.clone(), None, message);
      Self::bump_target_pub(&mut c, &Target::Root);
      let (tx, rx) = oneshot::channel();
      c.questions.pending.push(PendingQuestion { tx, block_id: block_id.clone(), questions, node_id: None, reply: QuestionReply::Caller });
      self.touch(&mut c);
      (rx, block_id)
    };
    match tokio::time::timeout(wait.min(ASK_MAX_WAIT), rx).await {
      Ok(r) => r.ok(),
      Err(_) => {
        self.close_question(&block_id, acpira_shared::transcript::QuestionOutcome::Cancelled);
        None
      }
    }
  }
}
