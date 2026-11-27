//! Local run on an SSH host (docs/dev/remote.md, "Built-in agent"): the session's folder is `ssh://example-host/<virtual
//! root>/work`, a path that does not exist here; the fake host's helper maps it to a temp folder. So every project
//! call has to go through the helper, and one that read this disk instead would fail

use super::*;
use crate::remote_connections::Lab;
use acpira_shared::remote::ConnectionOp;

const VIRT: &str = "/acpira-virtual-host";

struct Remote {
  env: Env,
  lab: Lab,
  real: PathBuf,
  connections: Arc<acpira_host::remote::connections::Connections>,
}

impl Remote {
  async fn new() -> Remote {
    let mut env = Env::new();
    let lab = Lab::new();
    let real = lab.path().join("hostfs");
    std::fs::create_dir_all(real.join("work")).unwrap();
    lab.map_root(VIRT, &real);
    let connections = lab.engine("engine");
    connections.op(ConnectionOp::Add { alias: "example-host".into() }).await.unwrap();
    connections.ensure("example-host", Duration::from_secs(30)).await.unwrap();
    env.deps.remote = Some(connections.clone());
    Remote { env, lab, real, connections }
  }

  /// `approval: full`: no permission card asks
  fn full_access(&self) {
    let path = self.env.root.path().join("providers.json");
    let mut p: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    p["approval"] = json!("full");
    std::fs::write(path, p.to_string()).unwrap();
  }

  async fn session(&self) -> Disposing {
    let s = Disposing(AcpSession::fresh("acpira", &format!("ssh://example-host{VIRT}/work"), self.env.deps.clone(), None));
    s.start().await;
    s
  }
}

impl Drop for Remote {
  fn drop(&mut self) {
    self.connections.dispose();
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_and_commands_run_on_the_host() {
  let r = Remote::new().await;
  assert!(!Path::new(VIRT).exists());
  let s = r.session().await;
  assert!(view(&s)["error"].is_null(), "{}", view(&s)["error"]);
  r.env.server.push(mock::tools(&[("c1", "write", json!({ "path": "hello.txt", "content": "hi from the host\n" }))]));
  r.env.server.push(mock::tools(&[("c2", "bash", json!({ "command": "cat hello.txt; pwd" }))]));
  r.env.server.push(mock::text("Done."));
  let cards = run(&s, "say hi").await;
  // The write asked, with the diff the host prepared
  assert_eq!(cards.len(), 2, "the write and the command ask: {cards:?}");
  assert_eq!(std::fs::read_to_string(r.real.join("work/hello.txt")).unwrap(), "hi from the host\n");
  let vw = view(&s);
  expect_match(last_turn(&vw), json!({ "role": "agent", "stop": "end_turn" }));
  let blocks = agent_blocks(&vw);
  expect_match(blocks.iter().find(|b| b["kind"] == "edit").unwrap(), json!({ "status": "completed", "diffStat": { "add": 1 } }));
  let sent = r.env.server.requests().last().unwrap().body["messages"].to_string();
  assert!(sent.contains("hi from the host"), "the command's output came back: {sent}");
  // The host's folder, as the session names it (the helper never shows its test map's real root)
  assert!(sent.contains(&format!("{VIRT}/work")), "it ran in the host's folder: {sent}");
  assert!(!sent.contains(&r.real.to_string_lossy().into_owned()), "{sent}");
  assert!(r.lab.calls().iter().any(|c| c == "start"));
}

/// Full access, so no card asks: the host's own `beforeEdit` still sees every write before it lands, and the
/// hooks run on the host, in its project folder
#[tokio::test(flavor = "multi_thread")]
async fn the_hosts_hooks_gate_every_write_there() {
  let r = Remote::new().await;
  r.full_access();
  let work = r.real.join("work");
  assert!(std::process::Command::new("git").arg("init").arg("-q").arg(&work).status().unwrap().success());
  std::fs::create_dir_all(work.join(".agents")).unwrap();
  std::fs::write(work.join(".agents/block-secret.sh"), "payload=$(cat)\ncase \"$payload\" in *secret.txt*) echo 'secrets stay out' >&2; exit 2;; esac\nexit 0\n").unwrap();
  std::fs::write(work.join(".agents/hooks.json"), r#"{ "beforeEdit": "sh .agents/block-secret.sh", "afterTurn": "touch .after-ran" }"#).unwrap();
  let s = r.session().await;
  r.env.server.push(mock::tools(&[("c1", "write", json!({ "path": "secret.txt", "content": "x" }))]));
  r.env.server.push(mock::tools(&[("c2", "write", json!({ "path": "ok.txt", "content": "fine" }))]));
  r.env.server.push(mock::text("Done."));
  // The gate sends the rejection back once the turn ends
  r.env.server.push(mock::text("Understood."));
  run(&s, "write both").await;
  until(|| r.env.server.requests().len() == 4 && !s.is_running(), 15_000).await;
  assert!(!work.join("secret.txt").exists(), "beforeEdit kept the write off the host");
  assert_eq!(std::fs::read_to_string(work.join("ok.txt")).unwrap(), "fine");
  let reqs = r.env.server.requests();
  assert!(reqs[1].body["messages"].to_string().contains("secrets stay out"), "the model got the hook's reason");
  assert!(work.join(".after-ran").exists(), "afterTurn ran in the host's project folder");
}

/// The prompt's AGENTS.md, environment and skills are the host's, read there as the session opens
#[tokio::test(flavor = "multi_thread")]
async fn instructions_and_skills_come_from_the_host() {
  let r = Remote::new().await;
  r.full_access();
  let work = r.real.join("work");
  std::fs::create_dir_all(work.join(".git")).unwrap();
  std::fs::write(work.join("AGENTS.md"), "Run make check on the host.").unwrap();
  let skill = work.join(".agents/skills/deploy");
  std::fs::create_dir_all(&skill).unwrap();
  std::fs::write(skill.join("SKILL.md"), "---\ndescription: Ship the build\n---\nRun ./ship.sh from the host.\n").unwrap();
  let s = r.session().await;
  r.env.server.push(mock::tools(&[("c1", "skill", json!({ "name": "deploy" }))]));
  r.env.server.push(mock::text("Done."));
  run(&s, "deploy it").await;
  let reqs = r.env.server.requests();
  let first = reqs[0].body.to_string();
  assert!(first.contains("Run make check on the host.") && first.contains(&format!("{VIRT}/work/AGENTS.md")), "{first}");
  assert!(first.contains("deploy: Ship the build"), "the skill tool lists the host's skill: {first}");
  assert!(!first.contains(&r.real.to_string_lossy().into_owned()), "{first}");
  let sent = reqs.last().unwrap().body["messages"].to_string();
  assert!(sent.contains("Run ./ship.sh from the host."), "the skill loaded from the host: {sent}");
}

/// `show_image` and `workspace_hooks` check the host's files there, through the hub; the image shown is the host's
#[tokio::test(flavor = "multi_thread")]
async fn shown_images_and_the_hooks_check_come_from_the_host() {
  let mut r = Remote::new().await;
  r.full_access();
  let roster = Arc::new(acpira_host::relay::roster::Roster::new(r.env.root.path()));
  let hub = acpira_host::relay::hub::RelayHub::start(roster, Arc::new(|_: &str| {})).await.unwrap();
  r.env.deps.host_mcp = Some(std::sync::Arc::new(acpira_host::host_mcp::HostMcp::new(env!("CARGO_BIN_EXE_acpira")).with_hub(hub)));
  let work = r.real.join("work");
  assert!(std::process::Command::new("git").arg("init").arg("-q").arg(&work).status().unwrap().success());
  std::fs::create_dir_all(work.join(".agents")).unwrap();
  std::fs::write(work.join(".agents/hooks.json"), r#"{ "afterTurn": "true" }"#).unwrap();
  let png = b"\x89PNG\r\n\x1a\nhost pixels";
  std::fs::write(work.join("shot.png"), png).unwrap();
  let video = b"\0\0\0\x18ftypmp42 host recording";
  std::fs::write(work.join("clip.mp4"), video).unwrap();
  let s = r.session().await;
  let shot = format!("{VIRT}/work/shot.png");
  r.env.server.push(mock::tools(&[("c1", "acpira_show_image", json!({ "path": shot })), ("c2", "acpira_workspace_hooks", json!({ "cwd": format!("{VIRT}/work") })), ("v", "acpira_show_video", json!({ "path": format!("{VIRT}/work/clip.mp4") }))]));
  r.env.server.push(mock::text("Shown."));
  run(&s, "show it").await;
  let sent = r.env.server.requests().last().unwrap().body["messages"].to_string();
  assert!(sent.contains("Shown to the user: shot.png"), "{sent}");
  assert!(sent.contains("is valid") && sent.contains("afterTurn: `true`"), "the hooks check ran on the host: {sent}");
  // The rows keep the host paths alone: the page reads the files from the host when it shows them (`stage_file`),
  // so a later packet of the call can never leave a row without its image
  let vw = view(&s);
  let rows = vw.to_string();
  assert!(rows.contains(&format!(r#""mimeType":"image/png","uri":"{shot}""#)), "the image row names the host file: {rows}");
  assert!(rows.contains("video/mp4"), "{rows}");
  assert!(!rows.contains("\"blob\""), "nothing was copied at show time: {rows}");
  let blobs = r.env.root.path().join("sessions");
  assert!(!walkdir(&blobs).into_iter().any(|p| std::fs::read(&p).is_ok_and(|b| b == png || b == video)));
  // What a paired device may stage: only the files the session shows
  assert!(s.shows_file(&shot) && s.shows_file(&format!("{VIRT}/work/clip.mp4")));
  assert!(!s.shows_file(&format!("{VIRT}/work/.agents/hooks.json")) && !s.shows_file("/etc/passwd"));
}

/// open_browser: the listen check runs on the host, and the shell is told the page is that folder's
#[tokio::test(flavor = "multi_thread")]
async fn open_browser_checks_the_port_on_the_host() {
  let mut r = Remote::new().await;
  r.full_access();
  let roster = Arc::new(acpira_host::relay::roster::Roster::new(r.env.root.path()));
  let hub = acpira_host::relay::hub::RelayHub::start(roster, Arc::new(|_: &str| {})).await.unwrap();
  let shell = Arc::new(Shell::default());
  hub.set_browser(shell.clone());
  r.env.deps.host_mcp = Some(std::sync::Arc::new(acpira_host::host_mcp::HostMcp::new(env!("CARGO_BIN_EXE_acpira")).with_hub(hub)));
  // A dev server on the host (this machine, for the fake one) and a port nothing serves there
  let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
  let up = server.local_addr().unwrap().port();
  let closed = { let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); l.local_addr().unwrap().port() };
  let s = r.session().await;
  r.env.server.push(mock::tools(&[
    ("c1", "acpira_open_browser", json!({ "url": format!("http://localhost:{up}/") })),
    ("c2", "acpira_open_browser", json!({ "url": format!("http://localhost:{closed}/") })),
  ]));
  r.env.server.push(mock::text("Opened."));
  run(&s, "open the app").await;
  let sent = r.env.server.requests().last().unwrap().body["messages"].to_string();
  assert!(sent.contains(&format!("Nothing is listening on localhost:{closed}")), "{sent}");
  assert_eq!(*shell.opened.lock(), [format!("http://localhost:{up}/ in ssh://example-host{VIRT}/work")]);
  drop(server);
}

/// A project MCP server the host's `.mcp.json` declares runs on the host, in the project there, through the
/// `acpira mcp-host` shim the agent starts here
#[tokio::test(flavor = "multi_thread")]
async fn a_project_mcp_server_runs_on_the_host() {
  let mut r = Remote::new().await;
  r.full_access();
  let roster = Arc::new(acpira_host::relay::roster::Roster::new(r.env.root.path()));
  let hub = acpira_host::relay::hub::RelayHub::start(roster, Arc::new(|_: &str| {})).await.unwrap();
  r.env.deps.host_mcp = Some(std::sync::Arc::new(acpira_host::host_mcp::HostMcp::new(env!("CARGO_BIN_EXE_acpira")).with_hub(hub)));
  let work = r.real.join("work");
  let server = r#"
const rl = require('readline').createInterface({ input: process.stdin });
const send = m => process.stdout.write(JSON.stringify(m) + '\n');
rl.on('line', l => {
  const m = JSON.parse(l);
  if (m.id === undefined) return;
  if (m.method === 'initialize') send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: m.params.protocolVersion, capabilities: { tools: {} }, serverInfo: { name: 'probe', version: '1' } } });
  else if (m.method === 'tools/list') send({ jsonrpc: '2.0', id: m.id, result: { tools: [{ name: 'where', description: 'Where the server runs', inputSchema: { type: 'object', properties: {} } }] } });
  else if (m.method === 'tools/call') send({ jsonrpc: '2.0', id: m.id, result: { content: [{ type: 'text', text: `cwd=${process.cwd()} mark=${process.env.MARK}` }] } });
  else send({ jsonrpc: '2.0', id: m.id, error: { code: -32601, message: 'no' } });
});
"#;
  std::fs::write(work.join("probe.js"), server).unwrap();
  std::fs::write(work.join(".mcp.json"), r#"{ "mcpServers": { "probe": { "command": "node", "args": ["probe.js"], "env": { "MARK": "from the host's file" } } } }"#).unwrap();
  let s = r.session().await;
  r.env.server.push(mock::tools(&[("c1", "probe_where", json!({}))]));
  r.env.server.push(mock::text("It runs there."));
  run(&s, "where does it run").await;
  let sent = r.env.server.requests().last().unwrap().body["messages"].to_string();
  let real = std::fs::canonicalize(&work).unwrap();
  assert!(sent.contains(&format!("cwd={}", real.display())) && sent.contains("mark=from the host's file"), "{sent}");
}

/// A shell that shows pages, noting the folder each one was for
#[derive(Default)]
struct Shell {
  opened: parking_lot::Mutex<Vec<String>>,
}

impl acpira_host::relay::hub::HostBrowser for Shell {
  fn available(&self) -> bool {
    true
  }
  fn open<'a>(&'a self, url: &'a str, folder: Option<&'a str>, _: Option<(&'a str, &'a str)>) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<acpira_shared::sidecar::BrowserOpened>> + Send + 'a>> {
    self.opened.lock().push(format!("{url} in {}", folder.unwrap_or("this machine")));
    Box::pin(async move { Ok(acpira_shared::sidecar::BrowserOpened { url: url.to_owned(), reused: None }) })
  }
}

fn walkdir(dir: &Path) -> Vec<PathBuf> {
  let mut out = vec![];
  for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
    let p = e.path();
    if p.is_dir() { out.extend(walkdir(&p)) } else { out.push(p) }
  }
  out
}

/// `/compact` brings back the files the turns read, as they are on the host now
#[tokio::test(flavor = "multi_thread")]
async fn compaction_restores_files_from_the_host() {
  let r = Remote::new().await;
  r.full_access();
  let notes = r.real.join("work/notes.txt");
  std::fs::write(&notes, "first draft\n").unwrap();
  let s = r.session().await;
  r.env.server.push(mock::tools(&[("c1", "read", json!({ "path": "notes.txt" }))]));
  r.env.server.push(mock::text("Read it."));
  run(&s, "read the notes").await;
  std::fs::write(&notes, "second draft from the host\n").unwrap();
  r.env.server.push(mock::text("The user had the notes read."));
  run(&s, "/compact").await;
  r.env.server.push(mock::text("Fine."));
  run(&s, "go on").await;
  let sent = r.env.server.requests().last().unwrap().body["messages"].to_string();
  assert!(sent.contains("second draft from the host"), "the restored file is the host's current one: {sent}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_cut_off_by_a_dropped_connection_is_reported_as_unknown() {
  let r = Remote::new().await;
  r.full_access();
  let s = r.session().await;
  r.env.server.push(mock::tools(&[("c1", "bash", json!({ "command": "echo started; sleep 31.4" }))]));
  r.env.server.push(mock::text("I will check first."));
  let task = spawn_prompt(&s, "run it");
  until(|| agent_blocks(&view(&s)).iter().any(|b| b["kind"] == "execute" && b["status"] == "in_progress"), 10_000).await;
  tokio::time::sleep(Duration::from_millis(300)).await;
  // The helper dies mid-command: its process group goes with it
  let pid = r.connections.client("example-host").unwrap().hello().pid as i32;
  // SAFETY: kill has no memory preconditions; the pid is the helper this test started
  unsafe { libc::kill(pid, libc::SIGKILL) };
  task.await.unwrap();
  let sent = r.env.server.requests().last().unwrap().body["messages"].to_string();
  assert!(sent.contains("unknown"), "the model is told the outcome is unknown: {sent}");
  expect_match(agent_blocks(&view(&s)).iter().find(|b| b["kind"] == "execute").unwrap(), json!({ "status": "failed" }));
}

/// A background job outlives its turn on the host, not its session: the close stops it while the helper stays up
#[tokio::test(flavor = "multi_thread")]
async fn closing_the_session_stops_its_jobs_on_the_host() {
  let r = Remote::new().await;
  r.full_access();
  let s = r.session().await;
  r.env.server.push(mock::tools(&[("c1", "bash", json!({ "command": "sleep 33.7", "background": true }))]));
  r.env.server.push(mock::text("Started."));
  run(&s, "start it").await;
  let running = || {
    let out = std::process::Command::new("ps").args(["-eo", "command"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().any(|l| l.contains("sleep 33.7"))
  };
  until(running, 5000).await;
  s.dispose();
  until(|| !running(), 15_000).await;
  assert!(r.connections.client("example-host").is_some(), "the connection stays up");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_turn_stops_the_command_on_the_host() {
  let r = Remote::new().await;
  r.full_access();
  let s = r.session().await;
  r.env.server.push(mock::tools(&[("c1", "bash", json!({ "command": "sleep 32.6" }))]));
  let task = spawn_prompt(&s, "run it");
  until(|| agent_blocks(&view(&s)).iter().any(|b| b["kind"] == "execute" && b["status"] == "in_progress"), 10_000).await;
  let running = || {
    let out = std::process::Command::new("ps").args(["-eo", "command"]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).lines().any(|l| l.contains("sleep 32.6"))
  };
  until(running, 5000).await;
  s.cancel().await;
  task.await.unwrap();
  until(|| !running(), 5000).await;
}
