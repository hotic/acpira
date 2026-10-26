//! `plugin_activate` (`docs/dev/plugins.md`, Activation): an agent of this session asks to enable a plugin it wrote.
//! The plugin is frozen and checked statically first (nothing of it runs), then the user decides on a card in this
//! session. The call waits as long as its client allows; a card answered after that still enables the plugin, and the
//! agent is told in a queued message

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use acpira_shared::transcript::{Question, QuestionKind, QuestionOption};

use crate::acp::session::AcpSession;
use crate::acp::session::gates::{PendingQuestion, QuestionReply, Target};
use crate::i18n::{t, tp};
use crate::plugins::host::{PluginHost, describe};
use crate::plugins::store::Provenance;
use crate::relay::wire::HubReply;

/// How long the call waits when the client named no bound (the 50 s every client but Claude gets, `host_mcp.rs`)
const DEFAULT_WAIT: Duration = Duration::from_secs(50);
const MAX_WAIT: Duration = Duration::from_secs(25 * 60);
const PROGRESS_EVERY: Duration = Duration::from_secs(5);
const ENABLE: &str = "enable";
const SKIP: &str = "skip";

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
    if host.store().load().ok().and_then(|s| s.plugins.get(&id).cloned()).is_some_and(|e| e.enabled && e.snapshot == frozen.hash) {
      let _ = out.send(HubReply::Done(format!("This version of {id} is enabled already.")));
      return;
    }
    let provenance = self.plugin_provenance();
    let agent = provenance.agent.clone().unwrap_or_else(|| self.agent.clone());
    let question = Question {
      id: "plugin".into(),
      title: None,
      text: tp("host.plugin.question", &[("name", frozen.manifest.display_name())]),
      kind: QuestionKind::Single,
      options: vec![
        QuestionOption { id: ENABLE.into(), label: t("host.plugin.enable"), description: None },
        QuestionOption { id: SKIP.into(), label: t("host.plugin.skip"), description: None },
      ],
      other: Some(false),
      numeric: None,
      required: None,
    };
    let message = format!("{}\n\n{}", tp("host.plugin.by", &[("agent", &agent)]), describe(&frozen));
    let mut rx = {
      let mut c = self.core.lock();
      let block_id = self.open_question(&mut c, &Target::Root, vec![question.clone()], None, Some(message));
      Self::bump_target_pub(&mut c, &Target::Root);
      let (tx, rx) = oneshot::channel();
      c.questions.pending.push(PendingQuestion { tx, block_id, questions: vec![question], node_id: None, reply: QuestionReply::Caller });
      self.touch(&mut c);
      rx
    };
    let deadline = wait_secs.map(Duration::from_secs).unwrap_or(DEFAULT_WAIT).min(MAX_WAIT);
    let until = tokio::time::Instant::now() + deadline;
    let mut tick = tokio::time::interval(PROGRESS_EVERY);
    tick.tick().await;
    let answer = loop {
      tokio::select! {
        r = &mut rx => break r.ok(),
        _ = tick.tick() => {
          if out.send(HubReply::Progress("Waiting for the user to enable the plugin".into())).is_err() {
            // The call was abandoned: the card stays, and its answer is told later
            break None;
          }
        }
        _ = tokio::time::sleep_until(until) => break None,
      }
    };
    if let Some(v) = answer {
      let text = self.plugin_answered(&host, &id, &frozen.hash, provenance, &v).await;
      let _ = out.send(text);
      return;
    }
    let _ = out.send(HubReply::Done(format!(
      "The card asking the user to enable {id} is still open. When they answer, you are told in a message; the plugin's \
       tools then appear in your tool list (in a new session for clients that do not list tools again). Go on with \
       other work meanwhile."
    )));
    // A late answer: enable and tell the agent in the next message
    let me = self.clone();
    tokio::spawn(async move {
      let Ok(v) = rx.await else { return };
      // Withdrawn with a stopped turn or a closed session: nobody to tell
      if v.get("cancelled").is_some() {
        return;
      }
      let reply = me.plugin_answered(&host, &id, &frozen.hash, provenance, &v).await;
      let text = match reply {
        HubReply::Done(t) | HubReply::Error(t) => t,
        _ => return,
      };
      me.enqueue(format!("[Acpira] {text}"), vec![], None).await;
    });
  }

  /// The card's answer: enable on yes (the dynamic check runs now), the text the agent is told either way
  async fn plugin_answered(&self, host: &PluginHost, id: &str, hash: &str, provenance: Provenance, answer: &Value) -> HubReply {
    if answer.get("cancelled").is_some() {
      return HubReply::Error("The card was withdrawn: the turn was stopped.".into());
    }
    let picked = answer.get("answers").and_then(|a| a.get("plugin")).and_then(Value::as_str).filter(|_| answer.get("skipped") != Some(&Value::Bool(true)));
    if picked != Some(ENABLE) {
      return HubReply::Done(format!("The user did not enable {id}. Do not try again unless they ask for it."));
    }
    match host.confirm(id, hash, provenance).await {
      Ok(_) => {
        let tools: Vec<String> = host.loaded().active.iter().filter(|a| a.id == id).flat_map(|a| a.manifest.contributes.tools.iter().map(|t| crate::plugins::manifest::served_name(id, &t.name))).collect();
        let tools = if tools.is_empty() { String::new() } else { format!(" Its tools on Acpira's MCP server: {}.", tools.join(", ")) };
        HubReply::Done(format!("The user enabled {id}.{tools} Clients that list tools again see them now; others in a new session."))
      }
      Err(e) => HubReply::Error(format!("The user agreed, but {id} was not enabled: {e:#}")),
    }
  }

  /// This session's agent and model, for the record of who asked
  fn plugin_provenance(&self) -> Provenance {
    let c = self.core.lock();
    let model = acpira_shared::model_shapes::session_model(&c.state.controls).map(|m| m.id);
    Provenance { by: "session".into(), session: Some(self.id.clone()), agent: Some(self.agent.clone()), model }
  }
}
