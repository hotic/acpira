//! A session folder on an SSH host as the pages read it (`workspace.rs`): the tool panel's listing, file, git marks and
//! session changes, and @ search, all through the helper behind the fake ssh. The host's files sit under a virtual root
//! that does not exist here, so a read that skipped the helper fails

#![cfg(unix)]

use std::time::Duration;

use acpira_host::project_files::ProjectError;
use acpira_host::workspace::Workspace;
use acpira_shared::remote::{ConnectionOp, RunMode};

use crate::remote_connections::Lab;

const VIRT: &str = "/acpira-virtual-ws";

#[tokio::test(flavor = "multi_thread")]
async fn the_tool_panel_reads_a_folder_on_the_host() {
  let lab = Lab::new();
  let real = lab.path().join("hostfs");
  let work = real.join("work");
  std::fs::create_dir_all(work.join("src")).unwrap();
  std::fs::write(work.join("src/a.rs"), "fn a() {}\n").unwrap();
  std::fs::write(work.join("README.md"), "old\n").unwrap();
  let git = |args: &[&str]| assert!(std::process::Command::new("git").arg("-C").arg(&work).args(args).output().unwrap().status.success(), "git {args:?}");
  git(&["init", "-q"]);
  git(&["-c", "user.email=t@t", "-c", "user.name=t", "add", "."]);
  git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]);
  std::fs::write(work.join("README.md"), "new\n").unwrap();
  std::fs::write(work.join("notes.txt"), "x\n").unwrap();
  lab.map_root(VIRT, &real);
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into(), mode: RunMode::Local }).await.unwrap();
  connections.ensure("gen1", Duration::from_secs(30)).await.unwrap();

  let ws = Workspace::open(&format!("ssh://gen1{VIRT}/work"), Some(&connections)).await.expect("connected");
  assert_eq!(ws.path(), std::path::Path::new(&format!("{VIRT}/work")));
  let names: Vec<(String, bool)> = ws.list_dir("").await.unwrap().into_iter().map(|e| (e.name, e.dir)).collect();
  assert_eq!(names, [("src".to_owned(), true), ("notes.txt".into(), false), ("README.md".into(), false)]);
  assert_eq!(ws.read_text("src/a.rs").await.unwrap(), "fn a() {}\n");
  assert_eq!(ws.read_text("../escape").await.err(), Some(ProjectError::Outside));
  assert_eq!(ws.read_text("gone.rs").await.err(), Some(ProjectError::Missing));
  assert_eq!(ws.head_text("README.md").await.as_deref(), Some("old\n"));
  let marks = ws.git_marks().await;
  assert_eq!((marks.get("README.md").map(String::as_str), marks.get("notes.txt").map(String::as_str)), (Some("M"), Some("U")));
  // An edit's base names the file by its path on the host, as the agent's tools do
  let changes = ws.session_changes(vec![(format!("{VIRT}/work/README.md"), Some("old\n".into()))]).await;
  assert_eq!((changes.len(), changes[0].path.as_str(), changes[0].add, changes[0].del), (1, "README.md", 1, 1));
  let hits = ws.search_files("a.rs").await;
  assert_eq!(hits.first().map(|h| h.path.as_str()), Some("src/a.rs"));
  assert!(hits[0].uri.contains(VIRT) && !hits[0].uri.contains(&*real.to_string_lossy()), "{}", hits[0].uri);
  // The handoff pack of the host's work tree (`ask_agent` with `changes`), built there
  let client = connections.client("gen1").unwrap();
  let pack = client.call("handoff", serde_json::json!({ "cwd": format!("{VIRT}/work") })).await.unwrap().value;
  let text = pack["pack"]["text"].as_str().unwrap_or("");
  assert!(text.contains("README.md") && text.contains("+new"), "{pack}");
  assert!(!text.contains(&*real.to_string_lossy()), "{text}");
  connections.dispose();
}

/// A terminal tab in a folder on the host runs its shell there (the helper's `pty`): input and output, the host's name
/// in the title, and a kill that ends the shell on the host
#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_in_a_folder_on_the_host_runs_there() {
  use acpira_shared::protocol::HostMsg;
  use std::sync::{Arc, Mutex};
  let lab = Lab::new();
  let real = lab.path().join("hostfs");
  std::fs::create_dir_all(real.join("work")).unwrap();
  lab.map_root(VIRT, &real);
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into(), mode: RunMode::Local }).await.unwrap();
  let hub = acpira_host::terminal::TerminalHub::new();
  hub.set_remote(connections.clone());
  let log: Arc<Mutex<Vec<HostMsg>>> = Arc::default();
  let l = log.clone();
  let sink: acpira_host::terminal::Sink = Arc::new(move |m| l.lock().unwrap().push(m));
  hub.open(1, sink, "t1", Some(&format!("ssh://gen1{VIRT}/work")), Some((100, 30)));
  let output = || {
    log.lock().unwrap().iter().map(|m| match m {
      HostMsg::TerminalOutput { data, .. } => data.clone(),
      HostMsg::TerminalState { replay: Some(r), .. } => r.clone(),
      _ => String::new(),
    }).collect::<String>()
  };
  hub.input("t1", "echo hi-$((40+2)); pwd; sleep 35.3\r".into(), false);
  crate::support::until(|| output().contains("hi-42") && output().contains("hostfs/work"), 20_000).await;
  // The ticker names the foreground program, with the host
  let titled = || log.lock().unwrap().iter().any(|m| matches!(m, HostMsg::TerminalState { title: Some(t), .. } if t.ends_with("(gen1)")));
  for _ in 0..100 {
    if titled() {
      break;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  let states: Vec<String> = log.lock().unwrap().iter().filter(|m| matches!(m, HostMsg::TerminalState { .. })).map(|m| format!("{m:?}")).collect();
  assert!(titled(), "{states:?}");
  let sleeping = || {
    let out = std::process::Command::new("ps").args(["-eo", "command"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().any(|l| l.contains("sleep 35.3"))
  };
  crate::support::until(sleeping, 5000).await;
  hub.kill("t1");
  crate::support::until(|| !sleeping(), 10_000).await;
  connections.dispose();
}

/// A dev server on the host, shown through the preview proxy: the proxy reaches it over the helper's `tcp`, and the same
/// origin on this machine gets a proxy of its own. A target off the host's loopback is refused there
#[tokio::test(flavor = "multi_thread")]
async fn a_preview_of_a_dev_server_on_the_host_goes_through_it() {
  use tokio::io::{AsyncReadExt, AsyncWriteExt};
  let lab = Lab::new();
  let connections = lab.engine("engine");
  connections.op(ConnectionOp::Add { alias: "gen1".into(), mode: RunMode::Local }).await.unwrap();
  connections.ensure("gen1", Duration::from_secs(30)).await.unwrap();
  // The fake host is this machine, so its loopback is this one's
  let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
  let up = listener.local_addr().unwrap().port();
  tokio::spawn(async move {
    while let Ok((mut s, _)) = listener.accept().await {
      tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        let mut head = vec![];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
          match s.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&buf[..n]),
          }
        }
        let body = "<html><head></head><body>on the host</body></html>";
        let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
        let _ = s.shutdown().await;
      });
    }
  });
  let picker = lab.path().join("picker.js");
  std::fs::write(&picker, "/* picker */").unwrap();
  let previews = acpira_host::preview::Previews::new(std::sync::Arc::new(|_: &str| {}));
  let url = format!("http://localhost:{up}/");
  let there = previews.open(1, "remote", &url, &picker, Some(("gen1".into(), connections.clone()))).await.unwrap();
  let here = previews.open(1, "local", &url, &picker, None).await.unwrap();
  assert_ne!(there.url, here.url, "one proxy per host and origin");
  let port: u16 = there.url.trim_start_matches("http://127.0.0.1:").split('/').next().unwrap().parse().unwrap();
  let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
  s.write_all(format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes()).await.unwrap();
  let mut out = vec![];
  tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut out)).await.unwrap().unwrap();
  let res = String::from_utf8_lossy(&out);
  assert!(res.starts_with("HTTP/1.1 200") && res.contains("on the host") && res.contains(&there.token), "{res}");

  let client = connections.client("gen1").unwrap();
  let refused = client.call("tcp", serde_json::json!({ "host": "10.255.0.1", "port": 80 })).await.expect_err("refused");
  assert!(refused.to_string().contains("loopback"), "{refused}");
  previews.close(1, "remote");
  previews.close(1, "local");
  connections.dispose();
}
