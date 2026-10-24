//! A page of the app (the chat or a browser panel) and its transport: the init script's channel down, `acpira_post`
//! batches up. Host messages go down as the JSON text they arrived in; nothing here parses a session view

use std::sync::Mutex;

use tauri::ipc::{Channel, InvokeResponseBody};

#[derive(Default)]
pub struct Transport {
  channel: Mutex<Option<Channel<InvokeResponseBody>>>,
}

impl Transport {
  /// The page (re)loaded and listens on a new channel; the old one belonged to a document that is gone
  pub fn attach(&self, channel: Channel<InvokeResponseBody>) {
    *self.channel.lock().unwrap() = Some(channel);
  }

  /// A navigation started: nothing listens until the new document attaches
  pub fn detach(&self) {
    *self.channel.lock().unwrap() = None;
  }

  /// One message, as JSON text; dropped while no document listens (as a webContents.send during a load would be)
  pub fn send(&self, json: String) {
    if let Some(c) = self.channel.lock().unwrap().as_ref() {
      let _ = c.send(InvokeResponseBody::Json(json));
    }
  }
}
