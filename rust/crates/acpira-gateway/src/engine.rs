//! How a device's envelope stream reaches the engine the gateway runs in (acpira-host `Engine::link`): one call per
//! stream gives the lines to the engine and the lines back. The way back ends with an empty line once the engine is done
//! with the wire, since a view of a running turn may still hold a sender then.

use std::sync::Arc;

use tokio::sync::mpsc;

pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

/// A new wire on the engine: (to the engine, from the engine)
pub type Link = Arc<dyn Fn() -> (mpsc::UnboundedSender<String>, mpsc::UnboundedReceiver<String>) + Send + Sync>;

/// A link to no engine: every wire ends at once (tests of pairing alone)
pub fn none() -> Link {
  Arc::new(|| {
    let (tx, _) = mpsc::unbounded_channel();
    let (_, rx) = mpsc::unbounded_channel();
    (tx, rx)
  })
}
