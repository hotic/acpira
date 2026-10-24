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
