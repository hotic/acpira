//! A cancellation signal (the AbortSignal of the TS code): cheap to clone, can be awaited, and a child fires when its
//! parent does

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct Cancel {
  inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
  fired: AtomicBool,
  notify: Notify,
  children: parking_lot::Mutex<Vec<Cancel>>,
}

impl Cancel {
  pub fn new() -> Self {
    Cancel::default()
  }

  pub fn cancel(&self) {
    if self.inner.fired.swap(true, Ordering::AcqRel) {
      return;
    }
    self.inner.notify.notify_waiters();
    let children = std::mem::take(&mut *self.inner.children.lock());
    for c in children {
      c.cancel();
    }
  }

  pub fn is_cancelled(&self) -> bool {
    self.inner.fired.load(Ordering::Acquire)
  }

  pub fn child(&self) -> Cancel {
    let c = Cancel::new();
    if self.is_cancelled() {
      c.cancel();
    } else {
      let mut kids = self.inner.children.lock();
      kids.retain(|k| !k.is_cancelled() && Arc::strong_count(&k.inner) > 1);
      kids.push(c.clone());
    }
    c
  }

  pub async fn cancelled(&self) {
    loop {
      let notified = self.inner.notify.notified();
      if self.is_cancelled() {
        return;
      }
      notified.await;
    }
  }
}
