//! The N-API layer (OpenHarmony only): each function wraps one `bridge::call`, async ones as Promises. acpira-mobile runs
//! its work on its own runtime and hands back futures any executor can poll; napi's runtime only awaits them and settles
//! the Promise on the ArkTS thread. A failure rejects with an `Error` whose message is the bridge's `Kind[: detail]`.

use std::sync::Arc;

use acpira_mobile as m;
use napi_derive_ohos::napi;
use napi_ohos::bindgen_prelude::Uint8Array;
use napi_ohos::{Error, Result, Status};

use crate::bridge::{self, Computer, Greeting, LinkEvent, NamedComputer, Relays, call};

fn told<T>(r: std::result::Result<T, String>) -> Result<T> {
  r.map_err(|message| Error::new(Status::GenericFailure, message))
}

/// The library's work is a handful of streams: napi's runtime gets one worker instead of one per core
#[napi_derive_ohos::module_init]
fn init() {
  if let Ok(rt) = tokio::runtime::Builder::new_multi_thread().worker_threads(1).thread_name("acpira-napi").enable_all().build() {
    napi_ohos::bindgen_prelude::create_custom_tokio_runtime(rt);
  }
}

/// A fresh identity key, hex, for the app to keep in its asset store
#[napi]
pub fn generate_key() -> String {
  bridge::key_hex(&m::generate_key())
}

/// The sidecar protocol this library's gateway counterpart speaks
#[napi]
pub fn protocol_version() -> i64 {
  m::protocol_version()
}

/// Reads a pairing link (or a bare ticket) without dialing anything
#[napi]
pub fn decode_link(link: String) -> Result<NamedComputer> {
  told(call::decode_link(link))
}

/// The text of the first QR code in a greyscale frame (a camera's Y plane, `stride` bytes per row), or null
#[napi]
pub fn decode_qr(grey: Uint8Array, width: u32, height: u32, stride: u32) -> Option<String> {
  bridge::decode_qr(&grey, width as usize, height as usize, stride as usize)
}

/// Binds a node with the device's identity (hex). It answers nothing: a device only dials
#[napi]
pub async fn start_node(secret: String, relays: Relays) -> Result<Node> {
  Ok(Node { inner: told(call::start_node(&secret, relays).await)? })
}

/// The device's endpoint; closing it ends every link on it
#[napi]
pub struct Node {
  inner: Arc<m::Node>,
}

#[napi]
impl Node {
  /// This device's EndpointId, as the computer lists it
  #[napi]
  pub fn id(&self) -> String {
    self.inner.id()
  }

  /// Whether the home relay is connected (true with relays off)
  #[napi]
  pub fn relay_connected(&self) -> bool {
    self.inner.relay_connected()
  }

  /// The system saw the network change: re-probe interfaces and paths now
  #[napi]
  pub fn network_changed(&self) {
    self.inner.network_changed();
  }

  /// Pair through a scanned QR code or a pasted link; `kind` is `phone` / `tablet` / `computer`
  #[napi]
  pub async fn pair_link(&self, link: String, name: String, kind: String, version: String, within_ms: u32) -> Result<NamedComputer> {
    told(call::pair_link(self.inner.clone(), link, name, &kind, version, within_ms).await)
  }

  /// Pair with a computer at known addresses and the 6-digit code its pairing card shows
  #[napi]
  pub async fn pair_code(&self, computer: Computer, code: String, name: String, kind: String, version: String, within_ms: u32) -> Result<NamedComputer> {
    told(call::pair_code(self.inner.clone(), computer, code, name, &kind, version, within_ms).await)
  }

  /// Opens a link to a paired computer's engine; the computer must answer and greet within `withinMs`
  #[napi]
  pub async fn connect(&self, computer: Computer, protocol_version: i64, within_ms: u32) -> Result<Link> {
    Ok(Link { inner: told(call::connect(self.inner.clone(), computer, protocol_version, within_ms).await)? })
  }

  /// Closes the endpoint and every link on it
  #[napi]
  pub async fn close(&self) {
    self.inner.close().await;
  }
}

/// A link to a paired computer's engine
#[napi]
pub struct Link {
  inner: Arc<m::Link>,
}

#[napi]
impl Link {
  #[napi]
  pub fn welcome(&self) -> Greeting {
    self.inner.welcome().into()
  }

  /// Sends envelopes: one JSON line, or several separated by newlines
  #[napi]
  pub async fn send(&self, line: String) -> Result<()> {
    told(call::send(self.inner.clone(), line).await)
  }

  /// The next envelope or link message; null once the link was closed from this side, the failure once it ended
  /// otherwise
  #[napi]
  pub async fn recv(&self) -> Result<Option<LinkEvent>> {
    told(call::recv(self.inner.clone()).await)
  }

  /// One blob of a session (an image its transcript shows), within the deadline
  #[napi]
  pub async fn fetch_blob(&self, session_id: String, name: String, within_ms: u32) -> Result<Uint8Array> {
    told(call::fetch_blob(self.inner.clone(), session_id, name, within_ms).await).map(Uint8Array::new)
  }

  /// One blob downloaded into the file at `path`, resuming after what the file holds; answers the blob's size. Ends
  /// only after `idleMs` without a byte
  #[napi]
  pub async fn fetch_blob_to(&self, session_id: String, name: String, path: String, idle_ms: u32) -> Result<i64> {
    told(call::fetch_blob_to(self.inner.clone(), session_id, name, path, idle_ms).await)
  }

  /// A round trip through the gateway in milliseconds; no answer within the deadline is `Offline`
  #[napi]
  pub async fn ping(&self, within_ms: u32) -> Result<u32> {
    told(call::ping(self.inner.clone(), within_ms).await)
  }

  /// The path the link travels on now (`lan` / `tailscale` / `direct` / `relay`), null before iroh picked one
  #[napi]
  pub fn path(&self) -> Option<String> {
    self.inner.path().map(bridge::path_name)
  }

  /// The next change of path; null once the link ended
  #[napi]
  pub async fn next_path(&self) -> Option<String> {
    call::next_path(self.inner.clone()).await
  }

  /// Ends the link; `recv` then resolves null
  #[napi]
  pub fn close(&self) {
    self.inner.close();
  }
}
