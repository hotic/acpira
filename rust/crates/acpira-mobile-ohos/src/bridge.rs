//! What crosses into ArkTS, free of N-API calls so the host builds and tests it. The records are plain objects on the
//! ArkTS side (`#[napi(object)]` on OpenHarmony only, field names in camelCase there); enums cross as strings, the
//! identity key as hex, and every failure as an `Error` whose message is `Kind` or `Kind: detail`, which the app reads
//! back with `failureKind` (apps/harmony/entry/src/main/ets/remote/Net.ets).

use acpira_mobile as m;

/// Who to dial (acpira-mobile `ComputerAddr`): a computer's EndpointId and where it was last seen
#[cfg_attr(target_env = "ohos", napi_derive_ohos::napi(object))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Computer {
  pub id: String,
  pub relay: Option<String>,
  pub addrs: Vec<String>,
}

/// A computer with its name: what a pairing link names (`LinkTicket`) and what a pairing answers (`Paired`)
#[cfg_attr(target_env = "ohos", napi_derive_ohos::napi(object))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedComputer {
  pub id: String,
  pub name: String,
  pub relay: Option<String>,
  pub addrs: Vec<String>,
}

/// The gateway's greeting on a new link
#[cfg_attr(target_env = "ohos", napi_derive_ohos::napi(object))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Greeting {
  pub name: String,
  pub version: String,
  pub protocol_version: i64,
}

/// One thing a link brought in (acpira-mobile `LinkEvent`): `kind` is `envelope` (with `line`), `addresses` (with
/// `relay` / `addrs`) or `denied` (with `what`)
#[cfg_attr(target_env = "ohos", napi_derive_ohos::napi(object))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkEvent {
  pub kind: String,
  pub line: Option<String>,
  pub relay: Option<String>,
  pub addrs: Option<Vec<String>>,
  pub what: Option<String>,
}

/// The last failed dial to a computer (acpira-mobile `DialReport`): `step` is `dial` (the computer never answered) or
/// `greeting` (it answered; its gateway then closed the link, stayed silent or spoke another protocol version)
#[cfg_attr(target_env = "ohos", napi_derive_ohos::napi(object))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialReport {
  pub step: String,
  pub detail: String,
  pub relay_connected: Option<bool>,
  pub took_ms: u32,
  pub version: Option<String>,
  pub protocol: Option<i64>,
}

/// Where traffic goes when no direct path works: `mode` is `n0`, `custom` (with `urls`, `token`) or `off`
#[cfg_attr(target_env = "ohos", napi_derive_ohos::napi(object))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relays {
  pub mode: String,
  pub urls: Option<Vec<String>>,
  pub token: Option<String>,
}

/// An unknown mode is n0's public relays, the app's default
pub fn relay_choice(r: Relays) -> m::RelayChoice {
  match r.mode.as_str() {
    "off" => m::RelayChoice::Off,
    "custom" => m::RelayChoice::Custom { urls: r.urls.unwrap_or_default(), token: r.token },
    _ => m::RelayChoice::N0,
  }
}

/// `phone`, `tablet` or `computer`; anything else is a phone
pub fn device_type(kind: &str) -> m::DeviceType {
  match kind {
    "tablet" => m::DeviceType::Tablet,
    "computer" => m::DeviceType::Computer,
    _ => m::DeviceType::Phone,
  }
}

pub fn path_name(p: m::LinkPath) -> String {
  match p {
    m::LinkPath::Lan => "lan",
    m::LinkPath::Tailscale => "tailscale",
    m::LinkPath::Direct => "direct",
    m::LinkPath::Relay => "relay",
  }
  .to_owned()
}

impl From<Computer> for m::ComputerAddr {
  fn from(c: Computer) -> Self {
    m::ComputerAddr { id: c.id, relay: c.relay, addrs: c.addrs }
  }
}

impl From<m::LinkTicket> for NamedComputer {
  fn from(t: m::LinkTicket) -> Self {
    NamedComputer { id: t.id, name: t.name, relay: t.relay, addrs: t.addrs }
  }
}

impl From<m::Paired> for NamedComputer {
  fn from(p: m::Paired) -> Self {
    NamedComputer { id: p.id, name: p.name, relay: p.relay, addrs: p.addrs }
  }
}

impl From<m::Welcome> for Greeting {
  fn from(w: m::Welcome) -> Self {
    Greeting { name: w.name, version: w.version, protocol_version: w.protocol_version }
  }
}

impl From<m::LinkEvent> for LinkEvent {
  fn from(e: m::LinkEvent) -> Self {
    let none = LinkEvent { kind: String::new(), line: None, relay: None, addrs: None, what: None };
    match e {
      m::LinkEvent::Envelope { line } => LinkEvent { kind: "envelope".into(), line: Some(line), ..none },
      m::LinkEvent::Addresses { relay, addrs } => LinkEvent { kind: "addresses".into(), relay, addrs: Some(addrs), ..none },
      m::LinkEvent::Denied { what } => LinkEvent { kind: "denied".into(), what: Some(what), ..none },
    }
  }
}

impl From<m::DialReport> for DialReport {
  fn from(r: m::DialReport) -> Self {
    let step = match r.step {
      m::DialStep::Dial => "dial",
      m::DialStep::Greeting => "greeting",
    };
    DialReport {
      step: step.to_owned(),
      detail: r.detail,
      relay_connected: r.relay_connected,
      took_ms: u32::try_from(r.took_ms).unwrap_or(u32::MAX),
      version: r.version,
      protocol: r.protocol,
    }
  }
}

/// `Kind` alone, or `Kind: detail` when the failure carries one
fn told(kind: &str, detail: Option<&str>) -> String {
  match detail {
    Some(d) => format!("{kind}: {d}"),
    None => kind.to_owned(),
  }
}

pub fn link_failure(f: &m::LinkFailure) -> String {
  match f {
    m::LinkFailure::Offline => told("Offline", None),
    m::LinkFailure::Revoked => told("Revoked", None),
    m::LinkFailure::Incompatible { message } => told("Incompatible", Some(message)),
    m::LinkFailure::Behind => told("Behind", None),
    m::LinkFailure::Engine { message } => told("Engine", Some(message)),
    m::LinkFailure::Other { message } => told("Other", Some(message)),
  }
}

pub fn pair_failure(f: &m::PairFailure) -> String {
  match f {
    m::PairFailure::Closed => told("Closed", None),
    m::PairFailure::Expired => told("Expired", None),
    m::PairFailure::Used => told("Used", None),
    m::PairFailure::Wrong => told("Wrong", None),
    m::PairFailure::InvalidLink => told("InvalidLink", None),
    m::PairFailure::Unreachable => told("Unreachable", None),
    m::PairFailure::Impostor => told("Impostor", None),
    m::PairFailure::Other { message } => told("Other", Some(message)),
  }
}

pub fn node_failure(f: &m::NodeFailure) -> String {
  match f {
    m::NodeFailure::BadKey => told("BadKey", None),
    m::NodeFailure::Other { message } => told("Other", Some(message)),
  }
}

/// The identity key as ArkTS keeps it (lowercase hex)
pub fn key_hex(key: &[u8]) -> String {
  key.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hex back to bytes; `None` for an odd length or a non-hex digit
pub fn key_bytes(hex: &str) -> Option<Vec<u8>> {
  let hex = hex.trim();
  if !hex.len().is_multiple_of(2) {
    return None;
  }
  hex.as_bytes().chunks(2).map(|pair| std::str::from_utf8(pair).ok().and_then(|s| u8::from_str_radix(s, 16).ok())).collect()
}

/// The text of the first QR code found in a greyscale frame (one byte per pixel, `stride` bytes per row: a camera's Y
/// plane), or `None`. A frame shorter than its size says is read as none. A frame with no code is read again with its
/// luma turned over: the desktop's dark theme draws the pairing code light on a dark card, which rqrr finds no grid in
/// (the Android scanner inverts its source the same way)
pub fn decode_qr(grey: &[u8], width: usize, height: usize, stride: usize) -> Option<String> {
  if width == 0 || height == 0 || stride < width || grey.len() < stride * (height - 1) + width {
    return None;
  }
  let luma = |x: usize, y: usize| grey.get(y * stride + x).copied().unwrap_or(u8::MAX);
  read_qr(width, height, luma).or_else(|| read_qr(width, height, |x, y| u8::MAX - luma(x, y)))
}

fn read_qr(width: usize, height: usize, luma: impl FnMut(usize, usize) -> u8) -> Option<String> {
  let mut img = rqrr::PreparedImage::prepare_from_greyscale(width, height, luma);
  img.detect_grids().iter().find_map(|g| g.decode().ok().map(|(_, text)| text))
}

/// The calls the N-API layer makes, one each, with failures already told as text (`napi.rs` only wraps them): the host
/// tests run exactly this against a gateway
pub mod call {
  use std::sync::Arc;

  use super::*;

  pub fn decode_link(link: String) -> Result<NamedComputer, String> {
    m::decode_link(link).map(NamedComputer::from).map_err(|e| pair_failure(&e))
  }

  pub async fn start_node(secret: &str, relays: Relays) -> Result<Arc<m::Node>, String> {
    let key = key_bytes(secret).ok_or_else(|| node_failure(&m::NodeFailure::BadKey))?;
    m::start_node(key, relay_choice(relays)).await.map_err(|e| node_failure(&e))
  }

  pub async fn pair_link(node: Arc<m::Node>, link: String, name: String, kind: &str, version: String, within_ms: u32) -> Result<NamedComputer, String> {
    let paired = node.pair_link(link, name, device_type(kind), version, within_ms.into()).await;
    paired.map(NamedComputer::from).map_err(|e| pair_failure(&e))
  }

  pub async fn pair_code(node: Arc<m::Node>, computer: Computer, code: String, name: String, kind: &str, version: String, within_ms: u32) -> Result<NamedComputer, String> {
    let paired = node.pair_code(computer.into(), code, name, device_type(kind), version, within_ms.into()).await;
    paired.map(NamedComputer::from).map_err(|e| pair_failure(&e))
  }

  pub async fn connect(node: Arc<m::Node>, computer: Computer, protocol_version: i64, within_ms: u32) -> Result<Arc<m::Link>, String> {
    node.connect(computer.into(), protocol_version, within_ms.into()).await.map_err(|e| link_failure(&e))
  }

  /// How the last dial to this computer failed; `None` once a dial opened a link, or before any failed
  pub fn dial_report(node: &m::Node, computer_id: String) -> Option<DialReport> {
    node.dial_report(computer_id).map(DialReport::from)
  }

  pub async fn send(link: Arc<m::Link>, line: String) -> Result<(), String> {
    link.send(line).await.map_err(|e| link_failure(&e))
  }

  pub async fn recv(link: Arc<m::Link>) -> Result<Option<LinkEvent>, String> {
    let next = link.recv().await.map_err(|e| link_failure(&e))?;
    Ok(next.map(LinkEvent::from))
  }

  pub async fn fetch_blob(link: Arc<m::Link>, session_id: String, name: String, within_ms: u32) -> Result<Vec<u8>, String> {
    link.fetch_blob(session_id, name, within_ms.into()).await.map_err(|e| link_failure(&e))
  }

  pub async fn fetch_blob_to(link: Arc<m::Link>, session_id: String, name: String, path: String, idle_ms: u32) -> Result<i64, String> {
    let size = link.fetch_blob_to(session_id, name, path, idle_ms.into(), Arc::new(Silent)).await.map_err(|e| link_failure(&e))?;
    Ok(i64::try_from(size).unwrap_or(i64::MAX))
  }

  pub async fn ping(link: Arc<m::Link>, within_ms: u32) -> Result<u32, String> {
    let rtt = link.ping(within_ms.into()).await.map_err(|e| link_failure(&e))?;
    Ok(u32::try_from(rtt).unwrap_or(u32::MAX))
  }

  pub async fn next_path(link: Arc<m::Link>) -> Option<String> {
    link.next_path().await.map(path_name)
  }

  /// Downloads report no progress to ArkTS: the file's length tells it
  struct Silent;

  impl m::BlobProgress for Silent {
    fn progress(&self, _have: u64, _total: u64) {}
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn keys_survive_hex() {
    let key = m::generate_key();
    let hex = key_hex(&key);
    assert_eq!(hex.len(), 64);
    assert_eq!(key_bytes(&hex), Some(key));
    assert_eq!(key_bytes("abc"), None);
    assert_eq!(key_bytes("zz"), None);
  }

  #[test]
  fn failures_name_their_kind_first() {
    assert_eq!(link_failure(&m::LinkFailure::Offline), "Offline");
    assert_eq!(link_failure(&m::LinkFailure::Incompatible { message: "update this app".into() }), "Incompatible: update this app");
    assert_eq!(pair_failure(&m::PairFailure::InvalidLink), "InvalidLink");
    assert_eq!(node_failure(&m::NodeFailure::BadKey), "BadKey");
  }

  #[test]
  fn events_carry_their_fields() {
    let e = LinkEvent::from(m::LinkEvent::Addresses { relay: None, addrs: vec!["10.0.2.2:1".into()] });
    assert_eq!(e.kind, "addresses");
    assert_eq!(e.addrs, Some(vec!["10.0.2.2:1".to_owned()]));
    assert_eq!(LinkEvent::from(m::LinkEvent::Envelope { line: "{}".into() }).line.as_deref(), Some("{}"));
  }

  #[test]
  fn dial_reports_name_their_step() {
    let unanswered = m::DialReport { step: m::DialStep::Dial, detail: "no answer within 15 s".into(), relay_connected: Some(false), took_ms: 15_004, version: None, protocol: None };
    let told = DialReport::from(unanswered);
    assert_eq!((told.step.as_str(), told.detail.as_str(), told.relay_connected, told.took_ms), ("dial", "no answer within 15 s", Some(false), 15_004));
    assert_eq!((told.version, told.protocol), (None, None));
    let greeted = m::DialReport { step: m::DialStep::Greeting, detail: "update this app".into(), relay_connected: None, took_ms: u64::MAX, version: Some("2.1.0".into()), protocol: Some(2) };
    let told = DialReport::from(greeted);
    assert_eq!((told.step.as_str(), told.relay_connected, told.took_ms), ("greeting", None, u32::MAX));
    assert_eq!((told.version.as_deref(), told.protocol), (Some("2.1.0"), Some(2)));
  }

  #[test]
  fn reads_a_code_from_a_padded_frame() {
    let link = "acpira://pair/ABCDEFGHIJKLMNOPQRSTUVWXYZ234567abcdefghijklmnop";
    let code = qrcode::QrCode::new(link).unwrap();
    let (modules, scale, quiet) = (code.width(), 6, 4);
    let colors = code.to_colors();
    // A camera-like frame: rows padded past the picture, the code off-centre on a light ground
    let (width, height, stride) = ((modules + 2 * quiet) * scale + 30, (modules + 2 * quiet) * scale + 10, (modules + 2 * quiet) * scale + 64);
    let mut grey = vec![230u8; stride * height];
    for y in 0..modules * scale {
      for x in 0..modules * scale {
        if colors[(y / scale) * modules + x / scale] == qrcode::Color::Dark {
          grey[(y + quiet * scale) * stride + x + quiet * scale + 20] = 20;
        }
      }
    }
    assert_eq!(decode_qr(&grey, width, height, stride).as_deref(), Some(link));
    // The desktop's dark theme draws the code light on a dark card
    let dark: Vec<u8> = grey.iter().map(|v| u8::MAX - v).collect();
    assert_eq!(decode_qr(&dark, width, height, stride).as_deref(), Some(link), "a light code on a dark ground");
    assert_eq!(decode_qr(&vec![230u8; stride * height], width, height, stride), None);
    assert_eq!(decode_qr(&grey[..10], width, height, stride), None, "a short frame is no frame");
  }

  #[test]
  fn strings_pick_enums() {
    assert_eq!(relay_choice(Relays { mode: "off".into(), urls: None, token: None }), m::RelayChoice::Off);
    assert_eq!(relay_choice(Relays { mode: "?".into(), urls: None, token: None }), m::RelayChoice::N0);
    assert_eq!(device_type("tablet"), m::DeviceType::Tablet);
    assert_eq!(path_name(m::LinkPath::Relay), "relay");
  }
}
