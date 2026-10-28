//! `acpira.context` and picked elements: what pages and browser panels share with the chat used last

use std::sync::Arc;

use acpira_desktop::browser::BrowserPage;
use acpira_desktop::context_api::{ContextPayloadError, add_context_msg, page_owner, parse_context_payload};
use acpira_desktop::wire::Initial;
use serde_json::{Value, json};

use super::App;

impl App {
  pub(super) fn share_context(&self) -> bool {
    self.settings.get("shareEditorSelection") != Some(Value::Bool(false))
  }

  pub(super) fn broadcast(&self, m: Value) {
    for w in self.windows() {
      w.post(m.clone());
    }
  }

  fn publish_context(&self, m: Option<Value>) {
    if let Some(m) = m
      && self.share_context()
    {
      self.broadcast(m);
    }
  }

  /// Picked elements go to the chat used last, like another tool's context; the chat comes up with them
  fn add_to_chat(self: &Arc<Self>, m: Value) {
    let Some(w) = self.focused_window().or_else(|| self.open(Some(Initial::MostRecent), self.place_of_last())) else { return };
    w.reveal();
    w.post(m);
  }

  pub fn add_picked(self: &Arc<Self>, item: Value) {
    match parse_context_payload(&json!({ "source": { "id": "browser", "name": self.t("browser.source", &[]) }, "items": [item] })) {
      Ok(p) => self.add_to_chat(add_context_msg(&p)),
      Err(e) => self.toast("error", &e.0, None),
    }
  }

  /// A page's own window.acpira.context call: what `set` resolves to (whether it shows), or the refused field
  pub fn page_context(self: &Arc<Self>, panel_id: &str, op: &str, raw: &Value) -> Result<Option<bool>, String> {
    let p = parse_context_payload(raw).map_err(|ContextPayloadError(e)| e)?;
    if op == "add" {
      if p.items.is_empty() {
        return Err("acpira.context: add needs at least one item".into());
      }
      self.add_to_chat(add_context_msg(&p));
      return Ok(None);
    }
    let owner = page_owner(panel_id, p.owner.as_deref());
    let m = self.context.lock().set(acpira_desktop::context_api::ContextPayload { owner: Some(owner), ..p });
    self.publish_context(m);
    Ok(Some(self.share_context()))
  }

  pub fn leave_page(&self, panel_id: &str) {
    let gone = self.context.lock().release_page(panel_id);
    for m in gone {
      self.publish_context(Some(m));
    }
  }

  pub fn visited(&self, page: BrowserPage) {
    self.history.visited(page);
    for b in self.browsers.lock().iter() {
      b.relay.post_history();
    }
    for w in self.windows() {
      w.post_browser_history();
    }
  }
}
