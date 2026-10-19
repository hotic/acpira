//! Preview proxies: what a page and its dev server see through one, and how leases open and close them

use std::sync::Arc;
use std::time::Duration;

use acpira_host::preview::Previews;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

const PICKER: &str = "/* picker */";

fn previews() -> (tempfile::TempDir, std::path::PathBuf, Arc<Previews>) {
  let dir = tempfile::tempdir().unwrap();
  let picker = dir.path().join("picker.js");
  std::fs::write(&picker, PICKER).unwrap();
  (dir, picker, Previews::new(Arc::new(|_: &str| {})))
}

/// A dev server that answers every connection with `respond(request head)`, written in the given pieces; the heads it got
/// come out of the channel
async fn upstream(respond: impl Fn(&str) -> Vec<Vec<u8>> + Send + Sync + 'static) -> (u16, mpsc::UnboundedReceiver<String>) {
  let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
  let port = listener.local_addr().unwrap().port();
  let (tx, rx) = mpsc::unbounded_channel();
  let respond = Arc::new(respond);
  tokio::spawn(async move {
    while let Ok((mut s, _)) = listener.accept().await {
      let (respond, tx) = (respond.clone(), tx.clone());
      tokio::spawn(async move {
        let head = read_head(&mut s).await;
        let _ = tx.send(head.clone());
        for piece in respond(&head) {
          if s.write_all(&piece).await.is_err() {
            return;
          }
          tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let _ = s.shutdown().await;
      });
    }
  });
  (port, rx)
}

async fn read_head(s: &mut TcpStream) -> String {
  let mut buf = vec![];
  let mut b = [0u8; 1];
  while !buf.ends_with(b"\r\n\r\n") && s.read(&mut b).await.unwrap_or(0) == 1 {
    buf.push(b[0]);
  }
  String::from_utf8_lossy(&buf).into_owned()
}

/// One raw request through the proxy; the whole response
async fn fetch(port: u16, request: &str) -> String {
  let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
  s.write_all(request.as_bytes()).await.unwrap();
  let mut out = vec![];
  tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await.unwrap().unwrap();
  String::from_utf8_lossy(&out).into_owned()
}

fn port_of(url: &str) -> u16 {
  url.trim_start_matches("http://127.0.0.1:").split('/').next().unwrap().parse().unwrap()
}

fn get(path: &str, port: u16) -> String {
  format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept-Encoding: gzip, br\r\n\r\n")
}

#[tokio::test]
async fn html_gets_the_picker_and_loses_what_would_keep_it_out_of_the_panel() {
  let page = r#"<!doctype html><html><head><meta http-equiv="content-security-policy" content="default-src 'none'"><title>t</title></head><body>hi</body></html>"#;
  let (up, _) = upstream(move |_| {
    vec![format!(
      "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nContent-Security-Policy: frame-ancestors 'none'\r\nX-Frame-Options: DENY\r\n\r\n{page}",
      page.len()
    )
    .into_bytes()]
  })
  .await;
  let (_d, picker, p) = previews();
  let o = p.open(1, "panel", &format!("http://127.0.0.1:{up}/blog?x=1"), &picker).await.unwrap();
  assert!(o.url.ends_with("/blog?x=1"), "{}", o.url);
  let port = port_of(&o.url);
  let res = fetch(port, &get("/", port)).await;
  let lower = res.to_ascii_lowercase();
  assert!(!lower.contains("content-security-policy") && !lower.contains("x-frame-options") && !lower.contains("content-length"), "{res}");
  assert!(lower.contains("connection: close"));
  assert!(res.ends_with(&format!(r#"<html><head><script src="/__acpira/picker.js?t={}"></script><title>t</title></head><body>hi</body></html>"#, o.token)), "{res}");
  let script = fetch(port, &get(&format!("/__acpira/picker.js?t={}", o.token), port)).await;
  assert!(script.starts_with("HTTP/1.1 200") && script.ends_with(PICKER), "{script}");
}

#[tokio::test]
async fn chunked_html_is_decoded_even_when_head_spans_two_chunks() {
  let (up, _) = upstream(|_| {
    let chunk = |s: &str| format!("{:x}\r\n{s}\r\n", s.len()).into_bytes();
    vec![
      b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec(),
      chunk("<html><he"),
      chunk("ad><title>a</title></head>"),
      chunk("<body>b</body></html>"),
      b"0\r\n\r\n".to_vec(),
    ]
  })
  .await;
  let (_d, picker, p) = previews();
  let o = p.open(1, "panel", &format!("http://localhost:{up}/"), &picker).await.unwrap();
  let port = port_of(&o.url);
  let res = fetch(port, &get("/", port)).await;
  assert!(!res.to_ascii_lowercase().contains("transfer-encoding"), "{res}");
  let body = res.split("\r\n\r\n").nth(1).unwrap();
  assert_eq!(body, format!(r#"<html><head><script src="/__acpira/picker.js?t={}"></script><title>a</title></head><body>b</body></html>"#, o.token));
}

#[tokio::test]
async fn upstream_sees_its_own_origin_and_redirects_and_cookies_come_back_for_the_panel() {
  let (up, mut heads) = upstream(move |head| {
    let next = format!("http://127.0.0.1:{}/next", head.lines().find(|l| l.starts_with("Host:")).unwrap().rsplit(':').next().unwrap().trim());
    vec![format!("HTTP/1.1 302 Found\r\nLocation: {next}\r\nSet-Cookie: sid=1; Path=/; HttpOnly\r\nSet-Cookie: t=2; SameSite=Strict\r\nContent-Length: 0\r\n\r\n").into_bytes()]
  })
  .await;
  let (_d, picker, p) = previews();
  let o = p.open(1, "panel", &format!("http://127.0.0.1:{up}/"), &picker).await.unwrap();
  let port = port_of(&o.url);
  let me = format!("http://127.0.0.1:{port}");
  let res = fetch(port, &format!("POST /login HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {me}\r\nReferer: {me}/form?a=1\r\nAccept-Encoding: gzip\r\nConnection: keep-alive\r\nContent-Length: 3\r\n\r\nx=1")).await;
  let head = heads.recv().await.unwrap();
  assert!(head.starts_with("POST /login HTTP/1.1\r\n"), "{head}");
  for want in [format!("Host: 127.0.0.1:{up}"), format!("Origin: http://127.0.0.1:{up}"), format!("Referer: http://127.0.0.1:{up}/form?a=1"), "Connection: close".into()] {
    assert!(head.contains(&want), "{want} missing in {head}");
  }
  assert!(!head.to_ascii_lowercase().contains("accept-encoding") && !head.contains("keep-alive"), "{head}");
  assert!(res.contains(&format!("Location: {me}/next")), "{res}");
  assert!(res.contains("Set-Cookie: sid=1; Path=/; HttpOnly; SameSite=None; Secure"), "{res}");
  assert!(res.contains("Set-Cookie: t=2; SameSite=None; Secure"), "{res}");
}

#[tokio::test]
async fn other_bodies_pass_byte_for_byte() {
  let binary: Vec<u8> = (0..=255u8).cycle().take(70_000).collect();
  let bytes = binary.clone();
  let (up, _) = upstream(move |_| {
    let mut res = format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n", bytes.len()).into_bytes();
    res.extend_from_slice(&bytes);
    vec![res]
  })
  .await;
  let (_d, picker, p) = previews();
  let o = p.open(1, "panel", &format!("http://127.0.0.1:{up}/"), &picker).await.unwrap();
  let port = port_of(&o.url);
  let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
  s.write_all(get("/bin", port).as_bytes()).await.unwrap();
  let mut out = vec![];
  s.read_to_end(&mut out).await.unwrap();
  let split = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
  assert!(String::from_utf8_lossy(&out[..split]).contains("Content-Length: 70000"));
  assert_eq!(&out[split..], &binary[..]);
}

#[tokio::test]
async fn event_streams_are_not_held_back() {
  // Upstream sends the second event only after the client has seen the first, which it only can if the proxy streams
  let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
  let up = listener.local_addr().unwrap().port();
  let (gate_tx, gate_rx) = tokio::sync::oneshot::channel::<()>();
  tokio::spawn(async move {
    let (mut s, _) = listener.accept().await.unwrap();
    read_head(&mut s).await;
    s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\ndata: one\n\n").await.unwrap();
    if tokio::time::timeout(Duration::from_secs(5), gate_rx).await.is_ok() {
      s.write_all(b"data: two\n\n").await.unwrap();
    }
  });
  let (_d, picker, p) = previews();
  let o = p.open(1, "panel", &format!("http://127.0.0.1:{up}/"), &picker).await.unwrap();
  let port = port_of(&o.url);
  let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
  s.write_all(get("/events", port).as_bytes()).await.unwrap();
  let mut seen = vec![];
  let mut buf = [0u8; 1024];
  tokio::time::timeout(Duration::from_secs(5), async {
    while !String::from_utf8_lossy(&seen).contains("data: one") {
      let n = s.read(&mut buf).await.unwrap();
      assert!(n > 0, "closed early");
      seen.extend_from_slice(&buf[..n]);
    }
  })
  .await
  .expect("the first event arrives while the stream is open");
  gate_tx.send(()).unwrap();
  s.read_to_end(&mut seen).await.unwrap();
  assert!(String::from_utf8_lossy(&seen).ends_with("data: one\n\ndata: two\n\n"));
}

#[tokio::test]
async fn websocket_upgrades_are_piped_both_ways() {
  let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
  let up = listener.local_addr().unwrap().port();
  let (tx, mut heads) = mpsc::unbounded_channel();
  tokio::spawn(async move {
    let (mut s, _) = listener.accept().await.unwrap();
    let _ = tx.send(read_head(&mut s).await);
    s.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n").await.unwrap();
    let mut buf = [0u8; 64];
    loop {
      let n = s.read(&mut buf).await.unwrap_or(0);
      if n == 0 {
        return;
      }
      s.write_all(&buf[..n]).await.unwrap();
    }
  });
  let (_d, picker, p) = previews();
  let o = p.open(1, "panel", &format!("http://127.0.0.1:{up}/"), &picker).await.unwrap();
  let port = port_of(&o.url);
  let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
  s.write_all(format!("GET /hmr HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nOrigin: http://127.0.0.1:{port}\r\n\r\n").as_bytes()).await.unwrap();
  let head = read_head(&mut s).await;
  assert!(head.starts_with("HTTP/1.1 101"), "{head}");
  let seen = heads.recv().await.unwrap();
  assert!(seen.contains("Connection: Upgrade") && seen.contains(&format!("Origin: http://127.0.0.1:{up}")), "{seen}");
  s.write_all(b"ping").await.unwrap();
  let mut buf = [0u8; 4];
  s.read_exact(&mut buf).await.unwrap();
  assert_eq!(&buf, b"ping");
}

#[tokio::test]
async fn a_dev_server_that_is_down_answers_502_and_other_hosts_are_refused() {
  let (_d, picker, p) = previews();
  let free = TcpListener::bind(("127.0.0.1", 0)).await.unwrap().local_addr().unwrap().port();
  let o = p.open(1, "panel", &format!("http://127.0.0.1:{free}/"), &picker).await.unwrap();
  let port = port_of(&o.url);
  assert!(fetch(port, &get("/", port)).await.starts_with("HTTP/1.1 502"));
  for bad in ["https://localhost:3000/", "http://example.com/", "http://192.168.1.2:80/"] {
    assert!(p.open(1, "panel", bad, &picker).await.is_err(), "{bad}");
  }
}

async fn listening(port: u16) -> bool {
  // A closed proxy stops accepting at once; give its task a moment to see the stop
  for _ in 0..50 {
    if TcpStream::connect(("127.0.0.1", port)).await.is_err() {
      return false;
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
  true
}

#[tokio::test]
async fn proxies_are_shared_by_target_and_close_with_their_last_lease() {
  let (a, _) = upstream(|_| vec![b"HTTP/1.1 204 No Content\r\n\r\n".to_vec()]).await;
  let (b, _) = upstream(|_| vec![b"HTTP/1.1 204 No Content\r\n\r\n".to_vec()]).await;
  let (_d, picker, p) = previews();
  let one = p.open(1, "p1", &format!("http://127.0.0.1:{a}/x"), &picker).await.unwrap();
  let two = p.open(2, "p9", &format!("http://127.0.0.1:{a}/y"), &picker).await.unwrap();
  assert_eq!((port_of(&one.url), &one.token), (port_of(&two.url), &two.token));
  let port = port_of(&one.url);
  // Window 1 reloads: window 2 still shows the page
  p.release_wire(1);
  assert!(listening(port).await);
  // Window 2's panel moves to another site: the first proxy has no one left
  let other = p.open(2, "p9", &format!("http://127.0.0.1:{b}/"), &picker).await.unwrap();
  assert!(!listening(port).await);
  assert_eq!(p.ports(), vec![port_of(&other.url)]);
  p.close(2, "p9");
  assert!(!listening(port_of(&other.url)).await);
  assert!(p.ports().is_empty());
}
