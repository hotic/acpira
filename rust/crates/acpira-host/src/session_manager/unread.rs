//! Unread marks: set when a reply ends unseen, cleared when any page of any engine lands on the session, and kept in
//! the shared store every engine reads

use std::collections::HashSet;

use super::SessionManager;
use crate::store::transcript_store::UnreadOp;

impl SessionManager {
  /// A reply nobody saw: marked here at once, then in the shared store where every other engine reads it
  pub(super) fn mark_unread(&self, id: &str) {
    if self.state.lock().unread.insert(id.to_owned()) {
      self.persist_unread(UnreadOp::Add(id.to_owned()));
    }
  }

  /// A page landed on the session, in any window of any engine: read everywhere
  pub(super) fn mark_read(&self, id: &str) {
    let removed = {
      let mut st = self.state.lock();
      st.kept_unread.remove(id);
      st.unread.remove(id)
    };
    if removed {
      self.persist_unread(UnreadOp::Remove(id.to_owned()));
    }
  }

  /// The row menu's "mark as unread / read". A session a page here shows keeps the mark until a page selects it again,
  /// where a reply's mark would be cleared at once
  pub(super) fn set_unread(&self, id: &str, unread: bool) {
    if unread {
      self.state.lock().kept_unread.insert(id.to_owned());
      self.mark_unread(id);
    } else {
      self.mark_read(id);
    }
    self.emit_sessions();
  }

  /// Unread marks reach the shared store in order, one batch at a time. Once the queue is drained the store's marks (other
  /// engines' included) become this engine's, and the other engines hear about the change
  pub(super) fn persist_unread(&self, op: UnreadOp) {
    let spawn = {
      let mut q = self.unread_ops.lock();
      q.ops.push(op);
      !std::mem::replace(&mut q.running, true)
    };
    if !spawn {
      return;
    }
    let weak = self.me.clone();
    tokio::spawn(async move {
      loop {
        let Some(m) = weak.upgrade() else { return };
        let ops: Vec<UnreadOp> = std::mem::take(&mut m.unread_ops.lock().ops);
        let written = m.deps.store.update_unread(&ops).await;
        let changed = {
          let mut q = m.unread_ops.lock();
          if !q.ops.is_empty() {
            continue;
          }
          q.running = false;
          // Taken over while the queue is locked, so a mark made meanwhile is not overwritten by the older store copy
          match &written {
            Ok(disk) => m.adopt_unread(disk),
            Err(e) => {
              m.log(&format!("unread marks not saved: {e}"));
              false
            }
          }
        };
        if written.is_ok() {
          m.bus.touch("unread");
        }
        m.read_shown();
        if changed {
          m.emit_sessions();
        }
        return;
      }
    });
  }

  /// The store's unread marks as this engine's; whether that changed anything
  fn adopt_unread(&self, disk: &std::collections::BTreeSet<String>) -> bool {
    let next: HashSet<String> = disk.iter().cloned().collect();
    let mut st = self.state.lock();
    if st.unread == next {
      return false;
    }
    // A kept mark another engine cleared is gone here too
    st.kept_unread.retain(|id| next.contains(id));
    st.unread = next;
    true
  }

  /// A mark another engine set on a session a page here shows (it could not see that page) is cleared again
  fn read_shown(&self) {
    let kept = self.state.lock().kept_unread.clone();
    let shown: Vec<String> = self.viewers().iter().filter_map(|v| v.active_id()).filter(|id| !kept.contains(id)).collect();
    for id in shown {
      self.mark_read(&id);
    }
  }

  /// Another engine changed the shared unread marks (or this one looks again): take them over. Skipped while this engine's
  /// own writes are queued, which end by taking the store's marks over anyway
  pub(super) async fn reload_unread(&self) {
    if self.unread_ops.lock().running {
      return;
    }
    let disk = self.deps.store.load_prefs().await.unread;
    let changed = {
      let q = self.unread_ops.lock();
      !q.running && self.adopt_unread(&disk)
    };
    self.read_shown();
    if changed {
      self.emit_sessions();
    }
  }

  /// "Mark all as read" reaches the SSH hosts in remote run too: their sessions' marks live on the host
  pub(super) fn mark_hosts_read(&self) {
    self.hosts_send_list(serde_json::json!({ "type": "markAllRead" }));
  }
}
