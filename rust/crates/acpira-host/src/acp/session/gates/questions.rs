//! Question cards: elicitation forms, Grok's questions and permissions asked as questions, answered from the page or by the caller that opened them

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Map, Value, json};
use tokio::sync::oneshot;

use acpira_shared::transcript::*;

use super::{ANSWER_IMAGES, MAX_ANSWER_IMAGES, PendingQuestion, QuestionReply, Target, cancel_reply, cancelled_permission, settle_question};
use crate::acp::transport::cancel::Cancel;
use crate::acp::transcript::normalize::activity_of;
use crate::acp::transcript::questions::{
  clean_answers, form_content, form_question_count, form_questions, grok_questions, grok_response, spare_message,
};
use crate::acp::session::{AcpSession, Core};
use crate::i18n::t;

impl AcpSession {
  // Question gate

  pub(crate) async fn on_elicitation(self: &Arc<Self>, req: Value, cancel: Cancel) -> Value {
    if cancel.is_cancelled() {
      return json!({ "action": "cancel" });
    }
    let schema = req.get("requestedSchema").filter(|s| s.is_object()).cloned();
    let tool_call_id = req.get("toolCallId").and_then(Value::as_str).map(str::to_owned);
    let session_id = req.get("sessionId").and_then(Value::as_str).map(str::to_owned);
    let (Some(schema), Some("form")) = (schema, req.get("mode").and_then(Value::as_str)) else { return json!({ "action": "decline" }) };
    let message = req.get("message").and_then(Value::as_str).unwrap_or("").to_owned();
    // Read before taking the core lock: relay_vendor locks it too
    let vendor = session_id.as_deref().and_then(|sid| self.relay_vendor(sid)).unwrap_or(self.vendor);
    let (rx, block_id) = {
      let mut c = self.core.lock();
      let Some(target) = self.target_for_call(&mut c, vendor, session_id.as_deref(), tool_call_id.as_deref()) else {
        drop(c);
        self.log(&format!("elicitation request for unknown session {}", session_id.as_deref().unwrap_or("(request scope)")));
        return json!({ "action": "cancel" });
      };
      let count = form_question_count(&schema);
      let raw = c.questions.raw.for_call(session_id.as_deref().unwrap_or(""), tool_call_id.as_deref(), count);
      let questions = form_questions(&schema, &message, req.get("_meta"), raw.as_deref());
      if questions.is_empty() {
        return json!({ "action": "decline" });
      }
      let spare = spare_message(&message, &questions);
      let images = req.pointer("/_meta/acpira~1answerImages") == Some(&Value::Bool(true));
      let block_id = self.open_question_with(&mut c, &target, questions.clone(), tool_call_id, spare, images);
      Self::bump_target(&mut c, &target);
      let (tx, rx) = oneshot::channel();
      c.questions.pending.push(PendingQuestion {
        tx,
        block_id: block_id.clone(),
        questions,
        node_id: target.node_id(),
        reply: QuestionReply::Form { schema, images },
      });
      self.touch(&mut c);
      (rx, block_id)
    };
    self.await_question(rx, block_id, cancel, json!({ "action": "cancel" })).await
  }

  pub(crate) async fn on_grok_question(self: &Arc<Self>, req: Value, cancel: Cancel) -> Value {
    if cancel.is_cancelled() {
      return json!({ "outcome": "skip_interview" });
    }
    let questions = grok_questions(&req);
    if questions.is_empty() {
      return json!({ "outcome": "accepted", "answers": {} });
    }
    let session_id = req.get("sessionId").and_then(Value::as_str).map(str::to_owned);
    let tool_call_id = req.get("toolCallId").and_then(Value::as_str).map(str::to_owned);
    let (rx, block_id) = {
      let mut c = self.core.lock();
      let Some(target) = Self::target_for(&mut c, session_id.as_deref()) else {
        drop(c);
        self.log(&format!("question request for unknown session {}", session_id.as_deref().unwrap_or("undefined")));
        return json!({ "outcome": "skip_interview" });
      };
      let block_id = self.open_question(&mut c, &target, questions.clone(), tool_call_id, None);
      Self::bump_target(&mut c, &target);
      let (tx, rx) = oneshot::channel();
      c.questions.pending.push(PendingQuestion {
        tx,
        block_id: block_id.clone(),
        questions,
        node_id: target.node_id(),
        reply: QuestionReply::Grok,
      });
      self.touch(&mut c);
      (rx, block_id)
    };
    self.await_question(rx, block_id, cancel, json!({ "outcome": "skip_interview" })).await
  }

  /// A permission request that is really a question (`Vendor::question_permission`): one single-choice question whose
  /// options are the request's, answered with the picked option id; skipping or closing the card cancels the request
  pub(super) async fn on_permission_question(self: &Arc<Self>, req: Value, tool_call_id: String, cancel: Cancel) -> Value {
    let tool_call = req.get("toolCall").cloned().unwrap_or(Value::Null);
    let options: Vec<QuestionOption> = req
      .get("options")
      .and_then(Value::as_array)
      .into_iter()
      .flatten()
      .filter_map(|o| {
        let id = o.get("optionId").and_then(Value::as_str).filter(|x| !x.is_empty())?.to_owned();
        let label = o.get("name").and_then(Value::as_str).filter(|x| !x.trim().is_empty()).unwrap_or(&id).to_owned();
        Some(QuestionOption { id, label, description: None })
      })
      .collect();
    if options.is_empty() {
      return cancelled_permission();
    }
    let session_id = req.get("sessionId").and_then(Value::as_str).map(str::to_owned);
    // Read before taking the core lock: relay_vendor locks it too
    let vendor = session_id.as_deref().and_then(|sid| self.relay_vendor(sid)).unwrap_or(self.vendor);
    let (rx, block_id) = {
      let mut c = self.core.lock();
      let Some(target) = self.target_for_call(&mut c, vendor, session_id.as_deref(), Some(&tool_call_id)) else {
        drop(c);
        self.log(&format!("question request for unknown session {}", session_id.as_deref().unwrap_or("undefined")));
        return cancelled_permission();
      };
      let text = tool_call
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| t("host.needApproval"));
      let questions = vec![Question {
        id: tool_call_id.clone(),
        title: None,
        text,
        kind: QuestionKind::Single,
        options,
        other: Some(false),
        numeric: None,
        required: None,
      }];
      let block_id = self.open_question(&mut c, &target, questions.clone(), Some(tool_call_id), None);
      Self::bump_target(&mut c, &target);
      let (tx, rx) = oneshot::channel();
      c.questions.pending.push(PendingQuestion {
        tx,
        block_id: block_id.clone(),
        questions,
        node_id: target.node_id(),
        reply: QuestionReply::Permission,
      });
      self.touch(&mut c);
      (rx, block_id)
    };
    self.await_question(rx, block_id, cancel, cancelled_permission()).await
  }

  async fn await_question(self: &Arc<Self>, rx: oneshot::Receiver<Value>, block_id: String, cancel: Cancel, fallback: Value) -> Value {
    // A summoned child asking while the root is idle: the automatic message carries the question now
    self.relay_nudge();
    tokio::select! {
      r = rx => r.unwrap_or(fallback),
      _ = cancel.cancelled() => {
        let mut c = self.core.lock();
        if let Some(i) = c.questions.pending.iter().position(|p| p.block_id == block_id) {
          let p = c.questions.pending.remove(i);
          settle_question(&mut c, &p, QuestionOutcome::Cancelled, None, None);
          self.touch(&mut c);
        }
        fallback
      }
    }
  }

  pub(crate) fn open_question(
    &self,
    c: &mut Core,
    target: &Target,
    questions: Vec<Question>,
    tool_call_id: Option<String>,
    message: Option<String>,
  ) -> String {
    self.open_question_with(c, target, questions, tool_call_id, message, false)
  }

  /// `images`: the card offers to attach images to the answers
  fn open_question_with(
    &self,
    c: &mut Core,
    target: &Target,
    questions: Vec<Question>,
    tool_call_id: Option<String>,
    message: Option<String>,
    images: bool,
  ) -> String {
    c.questions.seq += 1;
    let id = format!("q-{}", c.questions.seq);
    // A summoned child's question goes to the agent that summoned it, the user only once that one cannot answer
    // (`relay_escalate`); the card says the caller was asked too
    let summoned = matches!(target, Target::Node(n) if c.tree.relay_thread_of(n).is_some());
    let message = if summoned {
      c.questions.callers.insert(id.clone());
      c.relays.asked.send_modify(|n| *n += 1);
      Some(match message {
        Some(m) => format!("{m}\n\n{}", t("host.questionToCaller")),
        None => t("host.questionToCaller"),
      })
    } else {
      message
    };
    let block = QuestionBlock {
      id: id.clone(),
      tool_call_id: tool_call_id.filter(|x| !x.is_empty()),
      message,
      questions,
      outcome: None,
      answers: None,
      images: images.then_some(true),
      answer_images: None,
    };
    if let Some(state) = Self::target_state(c, target)
      && let Some(Turn::Agent(last)) = state.turns.last_mut()
    {
      last.blocks.push(AgentBlock::Question(block));
      let activity = activity_of(&state.turns);
      if let Some(Turn::Agent(last)) = state.turns.last_mut() {
        last.activity = activity;
      }
    }
    id
  }

  /// The card was closed: only answered questions travel; skip tells the agent to go on with what it has
  /// Whether the question card `block_id` still waits for its answer
  pub fn question_open(&self, block_id: &str) -> bool {
    self.core.lock().questions.pending.iter().any(|p| p.block_id == block_id)
  }

  pub fn answer_questions(&self, block_id: &str, answers: &QuestionAnswers, skip: bool) {
    self.answer_questions_with(block_id, answers, None, skip);
  }

  /// `answer_questions` with image drafts by question id, taken only by a form that asked for them
  pub fn answer_questions_with(&self, block_id: &str, answers: &QuestionAnswers, images: Option<&BTreeMap<String, Vec<Draft>>>, skip: bool) {
    let mut c = self.core.lock();
    let Some(i) = c.questions.pending.iter().position(|p| p.block_id == block_id) else { return };
    let p = c.questions.pending.remove(i);
    let mut given = clean_answers(&p.questions, answers);
    if matches!(p.reply, QuestionReply::Permission) {
      // Only an offered option can answer a permission request
      given.retain(|qid, a| p.questions.iter().any(|q| &q.id == qid && q.options.iter().any(|o| a.as_str() == Some(o.id.as_str()))));
    }
    let attached = match (&p.reply, images) {
      (QuestionReply::Form { images: true, .. }, Some(images)) if !skip => self.answer_images(&p.questions, images),
      _ => Vec::new(),
    };
    // An image alone answers its question
    let empty = given.is_empty() && attached.is_empty();
    let refs: BTreeMap<String, Vec<ImageRef>> = attached
      .iter()
      .filter_map(|(qid, list)| {
        let refs: Vec<ImageRef> = list.iter().filter_map(|i| Some(ImageRef { blob: Some(i.blob.clone()?), mime_type: i.mime.clone(), uri: None })).collect();
        (!refs.is_empty()).then(|| (qid.clone(), refs))
      })
      .collect();
    settle_question(
      &mut c,
      &p,
      if skip || empty { QuestionOutcome::Skipped } else { QuestionOutcome::Answered },
      Some(&given),
      (!refs.is_empty()).then_some(refs),
    );
    let reply = match &p.reply {
      QuestionReply::Grok => grok_response(skip, &given),
      QuestionReply::Permission => match given.values().next().and_then(Value::as_str).filter(|_| !skip) {
        Some(id) => json!({ "outcome": { "outcome": "selected", "optionId": id } }),
        None => cancelled_permission(),
      },
      QuestionReply::Form { schema, .. } => {
        if empty {
          json!({ "action": "decline" })
        } else {
          let mut reply = json!({ "action": "accept", "content": form_content(schema, &p.questions, &given) });
          if !attached.is_empty() {
            let by_question: Map<String, Value> = attached
              .iter()
              .map(|(qid, list)| (qid.clone(), list.iter().map(|i| json!({ "mimeType": i.mime, "data": i.data })).collect()))
              .collect();
            reply["_meta"] = json!({ ANSWER_IMAGES: by_question });
          }
          reply
        }
      }
      QuestionReply::Caller => json!({ "skipped": skip || empty, "answers": given }),
    };
    let _ = p.tx.send(reply);
    self.touch(&mut c);
  }

  /// A summoned child's own question (`ask_caller`) as a card on its running round: it routes like any question of that
  /// child, to its caller first (`open_question`)
  pub(crate) fn open_caller_question(&self, c: &mut Core, node: &str, questions: Vec<Question>) -> (oneshot::Receiver<Value>, String) {
    let target = Target::Node(node.to_owned());
    let block_id = self.open_question(c, &target, questions.clone(), None, None);
    Self::bump_target(c, &target);
    let (tx, rx) = oneshot::channel();
    c.questions.pending.push(PendingQuestion { tx, block_id: block_id.clone(), questions, node_id: Some(node.to_owned()), reply: QuestionReply::Caller });
    self.touch(c);
    (rx, block_id)
  }

  /// Close a card nobody answered (the asking call gave up or timed out), recording why
  pub(crate) fn close_question(&self, block_id: &str, outcome: QuestionOutcome) {
    let mut c = self.core.lock();
    let Some(i) = c.questions.pending.iter().position(|p| p.block_id == block_id) else { return };
    let p = c.questions.pending.remove(i);
    settle_question(&mut c, &p, outcome, None, None);
    self.touch(&mut c);
  }

  pub(crate) fn cancel_all_questions(&self, c: &mut Core) {
    let pending = std::mem::take(&mut c.questions.pending);
    for p in pending {
      settle_question(c, &p, QuestionOutcome::Cancelled, None, None);
      let reply = cancel_reply(&p);
      let _ = p.tx.send(reply);
    }
  }

  pub(crate) fn cancel_questions_for(&self, c: &mut Core, node_id: &str) {
    let mut i = 0;
    while i < c.questions.pending.len() {
      if c.questions.pending[i].node_id.as_deref() != Some(node_id) {
        i += 1;
        continue;
      }
      let p = c.questions.pending.remove(i);
      settle_question(c, &p, QuestionOutcome::Cancelled, None, None);
      let reply = cancel_reply(&p);
      let _ = p.tx.send(reply);
    }
  }
}

/// An image sent with an answer: its base64 for the agent and the blob it was kept as for the card's record
struct AnswerImage {
  mime: String,
  data: String,
  blob: Option<String>,
}

impl AcpSession {
  /// The image drafts that may travel with these questions' answers: known questions, image MIME types within the
  /// attachment limit, at most `MAX_ANSWER_IMAGES` per question. Each is kept in the blob store for the record
  fn answer_images(&self, questions: &[Question], images: &BTreeMap<String, Vec<Draft>>) -> Vec<(String, Vec<AnswerImage>)> {
    use base64::Engine;
    let save = crate::acp::session::images::image_saver(self.me.clone());
    let mut out = vec![];
    for q in questions {
      let Some(drafts) = images.get(&q.id) else { continue };
      let list: Vec<AnswerImage> = drafts
        .iter()
        .filter_map(|d| match d {
          Draft::Image { mime_type, data, .. } if mime_type.starts_with("image/") => Some((mime_type, data)),
          _ => None,
        })
        .filter(|(_, data)| base64::engine::general_purpose::STANDARD.decode(data.as_bytes()).is_ok_and(|b| b.len() <= acpira_shared::attachments::MAX_IMAGE_BYTES))
        .take(MAX_ANSWER_IMAGES)
        .map(|(mime, data)| AnswerImage { blob: save(data, mime), mime: mime.clone(), data: data.clone() })
        .collect();
      if !list.is_empty() {
        out.push((q.id.clone(), list));
      }
    }
    out
  }
}
