//! A dependency-free HTTP + WebSocket (RFC 6455, text frames) server for the browser harness: serves the harness page, the real webview bundle and attachment blobs, and turns each /ws
//! connection into a line channel. Development only; the IDE shells speak stdio

use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

pub struct HarnessOpts {
  pub port: u16,
  /// Repository root: test/host-preview/* and dist/webview/* are served from it
  pub root: PathBuf,
  /// Attachment blobs are served from here as /blobs/<sessionId>/<blob>
  pub sessions_dir: Arc<dyn Fn() -> Option<PathBuf> + Send + Sync>,
  pub token: Option<String>,
  pub log: Arc<dyn Fn(&str) + Send + Sync>,
}

/// One accepted WebSocket: lines from the page, a sender for lines to it
pub struct WsWire {
  pub lines: mpsc::UnboundedReceiver<String>,
  pub out: mpsc::UnboundedSender<String>,
}

fn mime(ext: &str) -> &'static str {
  match ext {
    "html" => "text/html; charset=utf-8",
    "js" | "mjs" => "text/javascript; charset=utf-8",
    "css" => "text/css; charset=utf-8",
    "json" => "application/json",
    "svg" => "image/svg+xml",
    "png" => "image/png",
    "jpg" | "jpeg" => "image/jpeg",
    "gif" => "image/gif",
    "webp" => "image/webp",
    "woff2" => "font/woff2",
    "woff" => "font/woff",
    "txt" => "text/plain; charset=utf-8",
    "md" => "text/markdown; charset=utf-8",
    "wasm" => "application/wasm",
    _ => "application/octet-stream",
  }
}

pub fn harness_origin_allowed(origin: Option<&str>, port: u16) -> bool {
  let Some(origin) = origin else { return false };
  let (scheme, rest) = match origin.split_once("://") {
    Some(x) => x,
    None => return false,
  };
  if scheme != "http" && scheme != "https" {
    return false;
  }
  let host_port = rest.split('/').next().unwrap_or("");
  let (host, p) = match host_port.rsplit_once(':') {
    Some((h, p)) => (h, p.parse::<u16>().ok()),
    None => (host_port, Some(if scheme == "https" { 443 } else { 80 })),
  };
  (host == "127.0.0.1" || host == "localhost") && p == Some(port)
}

/// Listen on 127.0.0.1; every accepted /ws connection is handed to `on_wire`. Resolves to the bound port
pub async fn start_harness(opts: HarnessOpts, on_wire: Arc<dyn Fn(WsWire) + Send + Sync>) -> std::io::Result<u16> {
  let listener = TcpListener::bind(("127.0.0.1", opts.port)).await?;
  let port = listener.local_addr()?.port();
  let q = opts.token.as_ref().map(|t| format!("/?token={t}")).unwrap_or_else(|| "/".into());
  (opts.log)(&format!("harness listening on http://127.0.0.1:{port}{q}"));
  let opts = Arc::new(opts);
  tokio::spawn(async move {
    while let Ok((stream, _)) = listener.accept().await {
      let (opts, on_wire) = (opts.clone(), on_wire.clone());
      tokio::spawn(async move {
        let _ = connection(stream, &opts, port, on_wire).await;
      });
    }
  });
  Ok(port)
}

struct Request {
  path: String,
  headers: Vec<(String, String)>,
}

impl Request {
  fn header(&self, name: &str) -> Option<&str> {
    self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
  }
}

async fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<(Request, Vec<u8>)>> {
  let mut buf = Vec::with_capacity(2048);
  let mut chunk = [0u8; 2048];
  loop {
    let n = stream.read(&mut chunk).await?;
    if n == 0 {
      return Ok(None);
    }
    buf.extend_from_slice(&chunk[..n]);
    if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
      let head = String::from_utf8_lossy(&buf[..end]).into_owned();
      let rest = buf[end + 4..].to_vec();
      let mut lines = head.split("\r\n");
      let first = lines.next().unwrap_or("");
      let path = first.split(' ').nth(1).unwrap_or("/").to_owned();
      let headers = lines.filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))).collect();
      return Ok(Some((Request { path, headers }, rest)));
    }
    if buf.len() > 64 * 1024 {
      return Ok(None);
    }
  }
}

async fn respond(stream: &mut TcpStream, status: &str, extra: &str, body: &[u8]) -> std::io::Result<()> {
  stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n", body.len()).as_bytes()).await?;
  stream.write_all(body).await?;
  stream.shutdown().await
}

fn query_param<'a>(path: &'a str, key: &str) -> Option<&'a str> {
  path.split_once('?')?.1.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
}

async fn connection(
  mut stream: TcpStream,
  opts: &HarnessOpts,
  port: u16,
  on_wire: Arc<dyn Fn(WsWire) + Send + Sync>,
) -> std::io::Result<()> {
  let Some((req, rest)) = read_request(&mut stream).await? else { return Ok(()) };
  let pathname = req.path.split('?').next().unwrap_or("/").to_owned();
  if req.header("upgrade").is_some_and(|u| u.eq_ignore_ascii_case("websocket")) {
    if pathname != "/ws" {
      return Ok(());
    }
    if let Some(token) = &opts.token
      && (query_param(&req.path, "token") != Some(token.as_str()) || !harness_origin_allowed(req.header("origin"), port))
    {
      return Ok(());
    }
    let Some(key) = req.header("sec-websocket-key") else { return Ok(()) };
    let accept =
      base64::engine::general_purpose::STANDARD.encode(crate::util::sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes()));
    stream
      .write_all(
        format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n")
          .as_bytes(),
      )
      .await?;
    ws_session(stream, rest, on_wire).await;
    return Ok(());
  }
  serve(&mut stream, &pathname, opts).await
}

/// Static files under a root, confined to it; directories are refused
async fn serve(stream: &mut TcpStream, pathname: &str, opts: &HarnessOpts) -> std::io::Result<()> {
  let Some(path) = percent_decode(pathname) else { return respond(stream, "400 Bad Request", "", b"").await };
  let (base, rel): (Option<PathBuf>, String) = if path == "/" || path == "/index.html" {
    (Some(opts.root.join("test").join("host-preview")), "index.html".into())
  } else if let Some(r) = path.strip_prefix("/host-preview/") {
    (Some(opts.root.join("test").join("host-preview")), r.into())
  } else if let Some(r) = path.strip_prefix("/webview/") {
    (Some(opts.root.join("dist").join("webview")), r.into())
  } else if let Some(r) = path.strip_prefix("/blobs/") {
    ((opts.sessions_dir)(), r.into())
  } else {
    (None, String::new())
  };
  let (Some(base), false) = (base, rel.is_empty()) else { return respond(stream, "404 Not Found", "", b"").await };
  let file = crate::store::data_dir::normalize(&base.join(&rel));
  if !file.starts_with(&base) || file == base {
    return respond(stream, "403 Forbidden", "", b"").await;
  }
  match tokio::fs::metadata(&file).await {
    Ok(m) if m.is_file() => {
      let body = tokio::fs::read(&file).await?;
      let ext = Path::new(&file).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
      respond(stream, "200 OK", &format!("Content-Type: {}\r\nCache-Control: no-store\r\n", mime(&ext)), &body).await
    }
    _ => respond(stream, "404 Not Found", "", b"").await,
  }
}

fn percent_decode(s: &str) -> Option<String> {
  let b = s.as_bytes();
  let mut out = Vec::with_capacity(b.len());
  let mut i = 0;
  while i < b.len() {
    if b[i] == b'%' {
      let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
      out.push(u8::from_str_radix(h, 16).ok()?);
      i += 3;
    } else {
      out.push(b[i]);
      i += 1;
    }
  }
  String::from_utf8(out).ok()
}

fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
  let len = payload.len();
  let mut out = vec![0x80 | opcode];
  if len < 126 {
    out.push(len as u8);
  } else if len < 65_536 {
    out.push(126);
    out.extend_from_slice(&(len as u16).to_be_bytes());
  } else {
    out.push(127);
    out.extend_from_slice(&(len as u64).to_be_bytes());
  }
  out.extend_from_slice(payload);
  out
}

/// Frames from the browser are masked; ours are not. Fragmented text is reassembled; ping gets pong; close gets close
async fn ws_session(stream: TcpStream, initial: Vec<u8>, on_wire: Arc<dyn Fn(WsWire) + Send + Sync>) {
  let (mut rd, mut wr) = stream.into_split();
  let (line_tx, line_rx) = mpsc::unbounded_channel::<String>();
  let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
  let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel::<Vec<u8>>();
  on_wire(WsWire { lines: line_rx, out: out_tx });
  let writer = tokio::spawn(async move {
    loop {
      tokio::select! {
        line = out_rx.recv() => match line {
          Some(l) => { if wr.write_all(&frame(0x1, l.as_bytes())).await.is_err() { break; } }
          None => { let _ = wr.write_all(&frame(0x8, &[0x03, 0xe8])).await; break; }
        },
        ctl = ctl_rx.recv() => match ctl {
          Some(bytes) => { if wr.write_all(&bytes).await.is_err() { break; } }
          None => break,
        },
      }
    }
    let _ = wr.shutdown().await;
  });
  let mut buf = initial;
  let mut fragments: Vec<u8> = vec![];
  let mut chunk = vec![0u8; 64 * 1024];
  'outer: loop {
    loop {
      if buf.len() < 2 {
        break;
      }
      let fin = buf[0] & 0x80 != 0;
      let opcode = buf[0] & 0x0f;
      let masked = buf[1] & 0x80 != 0;
      let mut len = (buf[1] & 0x7f) as usize;
      let mut off = 2;
      if len == 126 {
        if buf.len() < 4 {
          break;
        }
        len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
        off = 4;
      } else if len == 127 {
        if buf.len() < 10 {
          break;
        }
        len = u64::from_be_bytes(buf[2..10].try_into().unwrap()) as usize;
        off = 10;
      }
      let mask_len = if masked { 4 } else { 0 };
      if buf.len() < off + mask_len + len {
        break;
      }
      let mut payload = buf[off + mask_len..off + mask_len + len].to_vec();
      if masked {
        let mask = [buf[off], buf[off + 1], buf[off + 2], buf[off + 3]];
        for (i, b) in payload.iter_mut().enumerate() {
          *b ^= mask[i & 3];
        }
      }
      buf.drain(..off + mask_len + len);
      match opcode {
        0x8 => {
          let _ = ctl_tx.send(frame(0x8, &payload[..payload.len().min(2)]));
          break 'outer;
        }
        0x9 => {
          let _ = ctl_tx.send(frame(0xa, &payload));
        }
        0xa => {}
        0x0..=0x2 => {
          fragments.extend_from_slice(&payload);
          if !fin {
            continue;
          }
          let text = String::from_utf8_lossy(&fragments).into_owned();
          fragments.clear();
          // A text frame may carry several lines
          for line in text.split('\n').filter(|l| !l.trim().is_empty()) {
            let _ = line_tx.send(line.to_owned());
          }
        }
        _ => {}
      }
    }
    match rd.read(&mut chunk).await {
      Ok(0) | Err(_) => break,
      Ok(n) => buf.extend_from_slice(&chunk[..n]),
    }
  }
  drop(line_tx);
  drop(ctl_tx);
  let _ = writer.await;
}
