//! `plugin_activate` (`docs/dev/plugins.md`, Activation): an agent of this session offers a plugin it wrote. The
//! plugin is frozen and checked statically first (nothing of it runs), then a plugin card joins the agent's turn
//! (`AgentTurn::plugin_cards`) and the call returns at once, like showing an image: the agent's turn never waits on the
//! user. The card stays clickable after the turn; applying it is the user's confirmation and ends the exchange, and
//! undoing it puts back what it replaced. Only a failed apply goes back to the agent, as a queued message

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;

use acpira_shared::plugins::PluginCardOp;
use acpira_shared::transcript::{AgentBlock, AgentTurn, PluginCard, Turn};

use crate::acp::session::AcpSession;
use crate::acp::session::ext::{PluginCardPort, PluginOfferOutcome, Provenance};
use crate::acp::session::gates::{PendingQuestion, QuestionReply, Target};
use crate::i18n::tp;
use crate::relay_wire::HubReply;

impl AcpSession {
  pub async fn plugin_activate(self: &Arc<Self>, host: Arc<dyn PluginCardPort>, id: String) -> HubReply {
    let card_id = format!("plugin-{}", crate::util::random_hex(6));
    let card = match host.clone().prepare_offer(id.clone(), card_id.clone()).await {
      PluginOfferOutcome::Error(e) => return HubReply::Error(e),
      PluginOfferOutcome::AlreadyActive(msg) => return HubReply::Done(msg),
      PluginOfferOutcome::Offer(card) => *card,
    };
    {
      let mut c = self.core.lock();
      let Some(turn) = c.state.turns.iter_mut().rev().find_map(Turn::as_agent_mut) else {
        return HubReply::Error("There is no turn of yours to show the card in.".into());
      };
      let mut card = card;
      card.after = offering_call(turn);
      turn.plugin_cards.push(card);
      // The host the call came through, for a click in a session whose own hub has none
      self.plugin_offers.lock().insert(card_id, host.clone());
      self.touch(&mut c);
    }
    // The card's state reads the pages' plugin views, which may predate this plugin's folder
    host.offered();
    HubReply::Done(format!(
      "A card offering {id} is in your reply. The user applies it with one click whenever they like, also after this \
       turn; that click is the hand-off, so the plugin is delivered: do not wait for it and do not ask the user to \
       click. If applying it fails, you are told in a message with the error. Call plugin_activate again only after \
       changing the plugin."
    ))
  }

  /// A card's button: apply the version it offers, or undo what applying it changed. A failure is a toast; the
  /// card's state follows the plugins' state either way
  pub async fn plugin_card(self: &Arc<Self>, card_id: &str, op: PluginCardOp) {
    let own = self.deps.host_mcp.as_ref().and_then(|m| m.plugin_cards());
    let Some(host) = own.or_else(|| self.plugin_offers.lock().get(card_id).cloned()) else {
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
        }
        Err(e) => {
          let error = format!("{e:#}");
          self.notify(&tp("host.plugin.applyFailed", &[("name", &card.name), ("error", &error)]));
          // The version the agent handed over does not work: it hears why verbatim (a module that throws on load says
          // why only in the runtime's stderr), so it can fix the plugin and offer it again. A success needs no word:
          // the click ends the exchange, and new tools reach the agent through `tools/list_changed`
          self
            .enqueue(
              format!("[Acpira] The user applied the card, but {} was not enabled: {error}\nFix it and call plugin_activate again.", card.plugin),
              vec![],
              None,
            )
            .await;
        }
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
    self.host_ask(questions, message, wait).await
  }

  /// A question of the engine's own to the user, on a card in this session. The answer as a question card gives it
  /// (`answers`, `skipped`, `cancelled`), or None when `wait` passed first (the card is withdrawn)
  pub async fn host_ask(self: &Arc<Self>, questions: Vec<acpira_shared::transcript::Question>, message: Option<String>, wait: Duration) -> Option<Value> {
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

/// The row of the call that is offering a card: the turn's newest `plugin_activate` call, else its newest call still
/// running (the MCP request carries no tool call id, and the agent reports its call before making it). None: the card
/// shows after the reply
fn offering_call(turn: &AgentTurn) -> Option<String> {
  let calls = || turn.blocks.iter().rev().filter_map(|b| match b {
    AgentBlock::ToolCall(t) => Some(t),
    _ => None,
  });
  let named = |s: &str| s.to_ascii_lowercase().replace([' ', '-'], "_").contains("plugin_activate");
  calls()
    .find(|t| named(&t.verb) || t.target.as_deref().is_some_and(named))
    .or_else(|| calls().find(|t| t.status.is_open()))
    .map(|t| t.id.clone())
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn turn(blocks: Value) -> AgentTurn {
    serde_json::from_value(json!({ "role": "agent", "blocks": blocks })).unwrap()
  }
  fn call(id: &str, verb: &str, status: &str) -> Value {
    json!({ "type": "tool_call", "id": id, "kind": "other", "verb": verb, "status": status })
  }

  #[test]
  fn a_card_follows_the_call_that_offered_it() {
    // The activate call, even with a later call already running beside it
    let t = turn(json!([call("a", "Read", "completed"), call("b", "plugin_activate", "in_progress"), call("c", "Run", "in_progress")]));
    assert_eq!(offering_call(&t).as_deref(), Some("b"));
    // A client that names the call otherwise: the newest call still running
    let t = turn(json!([call("a", "Run", "in_progress"), call("b", "Read", "completed")]));
    assert_eq!(offering_call(&t).as_deref(), Some("a"));
    // No call at all: after the reply
    assert_eq!(offering_call(&turn(json!([{ "type": "text", "markdown": "hi" }]))), None);
  }
}
