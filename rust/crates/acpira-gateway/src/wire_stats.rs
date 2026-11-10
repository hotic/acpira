//! Local diagnostics for one device's envelope stream: frames and bytes each way, by message type, plus the QUIC
//! connection's own counters, written to the gateway's log (stderr, which the login service keeps in
//! `devices/gateway.log`). Nothing leaves the computer.
//!
//! - Payload bytes are the JSON as the engine and the device write it; wire bytes are what the envelope stream carried
//!   after its deflate codec. To-device frames count once the stream took them, so a frame and its wire bytes land in
//!   the same line.
//! - The link part (`link_line`) is the honest signal for a slow path: quinn's `write_all` returns as soon as the send
//!   window has room, so a relay's backlog sits inside QUIC (cwnd, losses, congestion events), not in the bridge.
//! - Blob streams (images) are not counted per type: they share the connection, so only the link part includes them.
//!   Tunnel streams (`tunnel.rs`) likewise; their own totals go in the device's disconnect line.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use acpira_net::endpoint;
use acpira_net::frame::FrameKind;
use acpira_net::iroh::endpoint::{Connection, PathId};
use acpira_shared::devices::PathKind;
use serde::Deserialize;

use crate::engine::Log;

/// How often a wire that moved anything logs what it moved
const EVERY: Duration = Duration::from_secs(60);
/// Types listed per direction, the most bytes first
const TOP: usize = 8;
/// Distinct types kept per tally; a device sending made-up types lands in `other` past it
const MAX_TYPES: usize = 64;
/// Envelopes not shaped the way the engine writes them (tag first) are parsed for their type only up to this size
const PARSE_LIMIT: usize = 64 << 10;
/// The engine's envelopes name their view message within the first bytes (`type`, `viewId`, then `message`)
const INNER_WITHIN: usize = 4 << 10;

#[derive(Default)]
struct Tally {
  /// Frames and payload bytes per type
  by_type: HashMap<String, (u64, u64)>,
  frames: u64,
  bytes: u64,
  largest: Option<(u64, String)>,
}

impl Tally {
  fn add(&mut self, ty: &str, n: u64) {
    self.frames += 1;
    self.bytes += n;
    if self.largest.as_ref().is_none_or(|(m, _)| n > *m) {
      self.largest = Some((n, ty.to_owned()));
    }
    let key = if self.by_type.contains_key(ty) || self.by_type.len() < MAX_TYPES { ty } else { "other" };
    // Looked up by `&str` first: no allocation for a type already counted
    if let Some(e) = self.by_type.get_mut(key) {
      e.0 += 1;
      e.1 += n;
    } else {
      self.by_type.insert(key.to_owned(), (1, n));
    }
  }

  /// `12 frames 3.0 KB (wire 1.9 KB), largest x 1.2 KB: x 10x 2.4 KB, y 2x 600 B`
  fn line(&self, wire: u64) -> String {
    let mut types: Vec<_> = self.by_type.iter().collect();
    types.sort_by(|a, b| b.1.1.cmp(&a.1.1).then_with(|| a.0.cmp(b.0)));
    let mut list: Vec<String> = types.iter().take(TOP).map(|(t, (f, b))| format!("{t} {f}x {}", size(*b))).collect();
    if types.len() > TOP {
      list.push(format!("+{} more", types.len() - TOP));
    }
    let largest = self.largest.as_ref().map(|(n, t)| format!(", largest {t} {}", size(*n))).unwrap_or_default();
    format!("{} frames {} (wire {}){largest}: {}", self.frames, size(self.bytes), size(wire), list.join(", "))
  }
}

/// One direction: since the last periodic line, and since the wire opened
#[derive(Default)]
struct Dir {
  window: Tally,
  total: Tally,
  /// Wire bytes as of the last periodic line
  wire_reported: u64,
}

impl Dir {
  fn add(&mut self, ty: &str, n: u64) {
    self.window.add(ty, n);
    self.total.add(ty, n);
  }
}

#[derive(Default)]
struct Inner {
  to_device: Dir,
  to_engine: Dir,
}

/// Counters for one envelope stream, fed by the bridge (`bridge::run`)
#[derive(Default)]
pub struct WireStats {
  inner: parking_lot::Mutex<Inner>,
  /// Bytes the stream carried each way, after the codec (`FrameWriter::written`, `FrameReader::read_bytes`)
  wire_out: AtomicU64,
  wire_in: AtomicU64,
}

impl WireStats {
  /// A frame the stream to the device took, with the bytes it has carried out so far
  pub fn to_device(&self, kind: FrameKind, body: &[u8], wire: u64) {
    let ty = frame_type(kind, body);
    self.inner.lock().to_device.add(&ty, body.len() as u64);
    self.wire_out.store(wire, Ordering::Relaxed);
  }

  /// A frame read from the device, with the bytes the stream has carried in so far
  pub fn to_engine(&self, kind: FrameKind, body: &[u8], wire: u64) {
    let ty = frame_type(kind, body);
    self.inner.lock().to_engine.add(&ty, body.len() as u64);
    self.wire_in.store(wire, Ordering::Relaxed);
  }

  /// What moved since the last call, or none when nothing did (no frame and no wire byte either way)
  fn window_line(&self) -> Option<String> {
    let (out, inn) = (self.wire_out.load(Ordering::Relaxed), self.wire_in.load(Ordering::Relaxed));
    let mut i = self.inner.lock();
    let (d, e) = (&i.to_device, &i.to_engine);
    if d.window.frames == 0 && e.window.frames == 0 && out == d.wire_reported && inn == e.wire_reported {
      return None;
    }
    let line = format!(
      "to device: {} | to engine: {}",
      d.window.line(out.saturating_sub(d.wire_reported)),
      e.window.line(inn.saturating_sub(e.wire_reported)),
    );
    i.to_device = Dir { window: Tally::default(), total: std::mem::take(&mut i.to_device.total), wire_reported: out };
    i.to_engine = Dir { window: Tally::default(), total: std::mem::take(&mut i.to_engine.total), wire_reported: inn };
    Some(line)
  }

  fn total_line(&self) -> String {
    let (out, inn) = (self.wire_out.load(Ordering::Relaxed), self.wire_in.load(Ordering::Relaxed));
    let i = self.inner.lock();
    format!("to device: {} | to engine: {}", i.to_device.total.line(out), i.to_engine.total.line(inn))
  }
}

/// The connection's counters at one moment, to log what changed since
#[derive(Clone, Copy, Default)]
struct LinkMark {
  /// UDP payload bytes each way, every path and stream of the connection (blobs too)
  udp_out: u64,
  udp_in: u64,
  sent_packets: u64,
  lost_packets: u64,
  lost_bytes: u64,
  /// The selected path and its congestion events: iroh counts those per path only
  congestion: Option<(PathId, u64)>,
}

/// `link: udp out 1.2 MB in 80.0 KB, lost 12/900 pkts (14.0 KB), congestion events 3, cwnd 38.0 KB, est 2.1 Mbit/s`
/// since `since`, with the mark to measure the next line from
fn link_line(conn: &Connection, since: &LinkMark) -> (String, LinkMark) {
  let c = conn.stats();
  let path = conn.paths().iter().find(|p| p.is_selected()).map(|p| (p.id(), p.stats()));
  let now = LinkMark {
    udp_out: c.udp_tx.bytes,
    udp_in: c.udp_rx.bytes,
    sent_packets: c.sent_packets,
    lost_packets: c.lost_packets,
    lost_bytes: c.lost_bytes,
    congestion: path.as_ref().map(|(id, s)| (*id, s.congestion_events)),
  };
  let mut line = format!(
    "link: udp out {} in {}, lost {}/{} pkts ({})",
    size(now.udp_out.saturating_sub(since.udp_out)),
    size(now.udp_in.saturating_sub(since.udp_in)),
    now.lost_packets.saturating_sub(since.lost_packets),
    now.sent_packets.saturating_sub(since.sent_packets),
    size(now.lost_bytes.saturating_sub(since.lost_bytes)),
  );
  if let Some((id, s)) = &path {
    // A path selected since the mark: its whole count
    let before = since.congestion.filter(|(was, _)| was == id).map_or(0, |(_, n)| n);
    line.push_str(&format!(", congestion events {}, cwnd {}", s.congestion_events.saturating_sub(before), size(s.cwnd)));
    if let Some(bps) = s.bandwidth_estimate {
      let mbps = bps as f64 / 1e6;
      line.push_str(&format!(", est {mbps:.1} Mbit/s"));
    }
  }
  (line, now)
}

/// Logs a wire's counters every `EVERY` while anything moves, and its totals when dropped: the bridge's task is aborted
/// when the device replaces its stream or the connection goes, so the last line cannot wait for `bridge::run` to return
pub struct Report {
  stats: Arc<WireStats>,
  conn: Connection,
  log: Log,
  label: String,
  since: Instant,
  /// The link's counters when the stream opened
  start: LinkMark,
  ticker: tokio::task::JoinHandle<()>,
}

impl Report {
  /// `label` names the device in every line
  pub fn start(stats: &Arc<WireStats>, conn: &Connection, log: &Log, label: String) -> Self {
    let start = link_line(conn, &LinkMark::default()).1;
    let ticker = {
      let (stats, conn, log, label) = (stats.clone(), conn.clone(), log.clone(), label.clone());
      tokio::spawn(async move {
        let mut every = tokio::time::interval_at(tokio::time::Instant::now() + EVERY, EVERY);
        // The link part covers the same span as the stream part: since the last line logged
        let mut mark = start;
        loop {
          every.tick().await;
          let Some(line) = stats.window_line() else { continue };
          let (link, now) = link_line(&conn, &mark);
          mark = now;
          let path = path_label(endpoint::selected_path(&conn));
          log(&format!("{label} since last on {path} | {link} | {line}"));
        }
      })
    };
    Self { stats: stats.clone(), conn: conn.clone(), log: log.clone(), label, since: Instant::now(), start, ticker }
  }
}

impl Drop for Report {
  fn drop(&mut self) {
    self.ticker.abort();
    let path = path_label(endpoint::selected_path(&self.conn));
    let (link, _) = link_line(&self.conn, &self.start);
    let line = self.stats.total_line();
    (self.log)(&format!("{} wire ended after {} s on {path} | {link} | {line}", self.label, self.since.elapsed().as_secs()));
  }
}

/// `relay, rtt 183 ms`, as the log names a selected path
pub fn path_label(p: Option<(PathKind, Duration)>) -> String {
  let Some((kind, rtt)) = p else { return "no path".into() };
  let kind = match kind {
    PathKind::Lan => "lan",
    PathKind::Tailscale => "tailscale",
    PathKind::Direct => "direct",
    PathKind::Relay => "relay",
  };
  format!("{kind}, rtt {} ms", rtt.as_millis())
}

/// The type a frame is counted under: a view message's own type inside its envelope (`sessionPatch`, `prompt`), else
/// the envelope's (`helloOk`, `attachView`); the link's own frames as `link:<type>`
fn frame_type(kind: FrameKind, body: &[u8]) -> Cow<'_, str> {
  let fast = || {
    let outer = quoted_at(body.strip_prefix(br#"{"type":""#)?)?;
    if !matches!(outer, "hostMessage" | "webviewMessage") {
      return Some(outer);
    }
    const INNER: &[u8] = br#""message":{"type":""#;
    let head = body.get(..INNER_WITHIN).unwrap_or(body);
    let at = head.windows(INNER.len()).position(|w| w == INNER)?;
    quoted_at(body.get(at + INNER.len()..)?)
  };
  let ty = match fast() {
    Some(t) => Cow::Borrowed(t),
    None => match parsed_type(body) {
      Some(t) => Cow::Owned(t),
      None => Cow::Borrowed("?"),
    },
  };
  match kind {
    FrameKind::Envelope => ty,
    // `link::Control`'s tags, spelled out so a ping costs no allocation
    FrameKind::Control => Cow::Borrowed(match &*ty {
      "ping" => "link:ping",
      "pong" => "link:pong",
      "push" => "link:push",
      "denied" => "link:denied",
      "welcome" => "link:welcome",
      "addresses" => "link:addresses",
      _ => "link:other",
    }),
  }
}

/// The JSON string starting at `s` (its opening quote already passed), when it is a plain name
fn quoted_at(s: &[u8]) -> Option<&str> {
  let end = s.iter().take(64).position(|&b| b == b'"')?;
  let name = std::str::from_utf8(s.get(..end)?).ok()?;
  (!name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'/' | b'.'))).then_some(name)
}

/// Slow path for small envelopes written in another key order (a device's encoder)
fn parsed_type(body: &[u8]) -> Option<String> {
  #[derive(Deserialize)]
  struct Inner {
    #[serde(rename = "type")]
    ty: Option<String>,
  }
  #[derive(Deserialize)]
  struct Head {
    #[serde(rename = "type")]
    ty: Option<String>,
    message: Option<Inner>,
  }
  if body.len() > PARSE_LIMIT {
    return None;
  }
  let h: Head = serde_json::from_slice(body).ok()?;
  match h.ty.as_deref() {
    Some("hostMessage" | "webviewMessage") => h.message.and_then(|m| m.ty).or(h.ty),
    _ => h.ty,
  }
}

pub(crate) fn size(n: u64) -> String {
  let f = n as f64;
  if n < 1 << 10 {
    format!("{n} B")
  } else if n < 1 << 20 {
    format!("{:.1} KB", f / 1024.0)
  } else {
    format!("{:.1} MB", f / (1024.0 * 1024.0))
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use acpira_shared::protocol::HostMsg;
  use acpira_shared::sidecar::SidecarMsg;

  #[test]
  fn frames_are_counted_under_their_view_message_type() {
    let engine = serde_json::to_string(&SidecarMsg::HostMessage {
      view_id: "V".into(),
      message: HostMsg::TerminalOutput { id: "t".into(), data: "x".repeat(10_000) },
    })
    .unwrap();
    assert_eq!(frame_type(FrameKind::Envelope, engine.as_bytes()), "terminalOutput");
    assert_eq!(frame_type(FrameKind::Envelope, br#"{"type":"helloOk","sessionsDir":"/x"}"#), "helloOk");
    // A device's encoder may put the tag anywhere
    assert_eq!(frame_type(FrameKind::Envelope, br#"{ "viewId": "V", "message": {"text": "hi", "type": "prompt"}, "type": "webviewMessage" }"#), "prompt");
    assert!(matches!(frame_type(FrameKind::Control, br#"{"type":"pong","n":3}"#), Cow::Borrowed("link:pong")));
    assert_eq!(frame_type(FrameKind::Control, br#"{"type":"later","n":3}"#), "link:other");
    assert_eq!(frame_type(FrameKind::Envelope, b"not json"), "?");
  }

  #[test]
  fn a_window_line_comes_only_when_something_moved_and_totals_keep_everything() {
    let s = WireStats::default();
    assert!(s.window_line().is_none());
    s.to_device(FrameKind::Envelope, br#"{"type":"hostMessage","viewId":"V","message":{"type":"sessionPatch","x":"aaaa"}}"#, 50);
    s.to_device(FrameKind::Envelope, br#"{"type":"helloOk"}"#, 70);
    s.to_engine(FrameKind::Control, br#"{"type":"ping","n":1}"#, 30);
    let line = s.window_line().unwrap();
    assert!(line.starts_with("to device: 2 frames 98 B (wire 70 B), largest sessionPatch 80 B: sessionPatch 1x 80 B, helloOk 1x 18 B"), "{line}");
    assert!(line.contains(" | to engine: 1 frames 21 B (wire 30 B), largest link:ping 21 B: link:ping 1x 21 B"), "{line}");
    assert!(s.window_line().is_none());
    // Wire bytes alone (frames sent before the hello, outside the counters) still make a line
    s.wire_out.store(75, Ordering::Relaxed);
    assert!(s.window_line().unwrap().starts_with("to device: 0 frames 0 B (wire 5 B)"));
    s.to_device(FrameKind::Envelope, br#"{"type":"helloOk"}"#, 100);
    assert!(s.window_line().unwrap().starts_with("to device: 1 frames 18 B (wire 25 B)"));
    assert!(s.total_line().starts_with("to device: 3 frames 116 B (wire 100 B), largest sessionPatch 80 B"));
  }
}
