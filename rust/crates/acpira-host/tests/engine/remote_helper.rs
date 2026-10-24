//! `acpira remote --stdio` through `RemoteClient`, without ssh. The host side is a virtual root (`/remote-root`) the
//! helper maps to a temp dir, so anything that touched the local disk instead of going through the helper fails

#![cfg(unix)]

use std::path::Path;
use std::time::Duration;

use acpira_host::remote::client::{ClientOpts, Event, RemoteClient, RemoteError};
use serde_json::json;
use tokio::process::Command;

const ROOT: &str = "/remote-root";

fn helper_cmd(real_root: &Path, extra: &[&str]) -> Command {
  let mut cmd = Command::new(env!("CARGO_BIN_EXE_acpira"));
  cmd.args(["remote", "--stdio", "--no-login-path"]).args(extra);
  cmd.env("ACPIRA_REMOTE_ROOT_MAP", format!("{ROOT}={}", real_root.display()));
  cmd
}

async fn start(real_root: &Path) -> RemoteClient {
  RemoteClient::start(helper_cmd(real_root, &[]), ClientOpts::default()).await.expect("helper starts")
}

fn alive(pid: i32) -> bool {
  // SAFETY: signal 0 only checks the process exists
  unsafe { libc::kill(pid, 0) == 0 }
}

async fn gone_within(pid: i32, limit: Duration) -> bool {
  let until = tokio::time::Instant::now() + limit;
  while tokio::time::Instant::now() < until {
    if !alive(pid) {
      return true;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
  }
  !alive(pid)
}

#[tokio::test]
async fn the_handshake_is_found_behind_login_shell_noise() {
  let dir = tempfile::tempdir().unwrap();
  let mut cmd = Command::new("/bin/sh");
  cmd.arg("-c").arg(format!("echo 'Welcome to Ubuntu'; echo '{{\"t\":\"res\",\"id\":1}}'; exec '{}' remote --stdio --no-login-path --digest abc", env!("CARGO_BIN_EXE_acpira")));
  cmd.env("ACPIRA_REMOTE_ROOT_MAP", format!("{ROOT}={}", dir.path().display())).env("ACPIRA_REMOTE_MACHINE_ID", "machine-a");
  let client = RemoteClient::start(cmd, ClientOpts::default()).await.unwrap();
  let hello = client.hello();
  assert_eq!(hello.machine_id, "machine-a");
  assert_eq!(hello.digest, "abc");
  assert_eq!(hello.protocol, acpira_host::remote::wire::PROTOCOL);
  // The fake answer before the handshake went nowhere: the first real request still gets its own answer
  let r = client.call("stat", json!({ "path": ROOT })).await.unwrap();
  assert_eq!(r.value["kind"], "dir");
}

#[tokio::test]
async fn a_transport_that_never_says_hello_reports_its_stderr() {
  let mut cmd = Command::new("/bin/sh");
  cmd.arg("-c").arg("echo 'Host key verification failed.' >&2; exit 255");
  let err = RemoteClient::start(cmd, ClientOpts::default()).await.err().expect("no helper");
  assert_eq!(err.exit_code, Some(255));
  assert_eq!(err.message, "Host key verification failed.");
}

#[tokio::test]
async fn files_go_through_the_virtual_root_only() {
  let dir = tempfile::tempdir().unwrap();
  let client = start(dir.path()).await;
  let file = format!("{ROOT}/src/a.txt");
  let w = client.call("write", json!({ "path": file, "b": acpira_host::remote::wire::b64(b"one\n"), "mkdirs": true })).await.unwrap();
  assert_eq!(w.value["created"], true);
  assert_eq!(std::fs::read_to_string(dir.path().join("src/a.txt")).unwrap(), "one\n");

  let r = client.call("read", json!({ "path": file })).await.unwrap();
  assert_eq!(r.data, b"one\n");
  let window = client.call("read", json!({ "path": file, "offset": 1, "len": 2 })).await.unwrap();
  assert_eq!(window.data, b"ne");

  let listed = client.call("list", json!({ "path": format!("{ROOT}/src") })).await.unwrap();
  assert_eq!(listed.value["entries"][0]["name"], "a.txt");

  client.call("rename", json!({ "from": file, "to": format!("{ROOT}/src/b.txt") })).await.unwrap();
  assert!(dir.path().join("src/b.txt").exists());
  client.call("remove", json!({ "path": format!("{ROOT}/src"), "recursive": true })).await.unwrap();
  assert!(!dir.path().join("src").exists());

  // The real location of the same folder is not reachable by its local name, and nothing outside the root is
  let real = client.call("stat", json!({ "path": dir.path().to_string_lossy() })).await.unwrap_err();
  assert_eq!(real.code(), "denied");
  let missing = client.call("read", json!({ "path": format!("{ROOT}/nope") })).await.unwrap_err();
  assert_eq!(missing.code(), "notFound");
  let relative = client.call("stat", json!({ "path": "src" })).await.unwrap_err();
  assert_eq!(relative.code(), "invalid");
}

#[tokio::test]
async fn writes_keep_permission_bits_and_refuse_a_changed_file() {
  use std::os::unix::fs::PermissionsExt;
  let dir = tempfile::tempdir().unwrap();
  let real = dir.path().join("run.sh");
  std::fs::write(&real, "#!/bin/sh\n").unwrap();
  std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o750)).unwrap();
  let client = start(dir.path()).await;
  let path = format!("{ROOT}/run.sh");
  let stat = client.call("stat", json!({ "path": path })).await.unwrap().value;
  let mtime = stat["mtimeMs"].as_u64().unwrap();

  let ok = client.call("write", json!({ "path": path, "b": acpira_host::remote::wire::b64(b"#!/bin/sh\necho hi\n"), "ifMtimeMs": mtime })).await;
  assert!(ok.is_ok(), "{ok:?}");
  assert_eq!(std::fs::metadata(&real).unwrap().permissions().mode() & 0o777, 0o750);

  // The file moved on since `mtime`: the stale write is refused and the file is left alone
  let stale = client.call("write", json!({ "path": path, "b": acpira_host::remote::wire::b64(b"old\n"), "ifMtimeMs": mtime.saturating_sub(5000) })).await.unwrap_err();
  assert_eq!(stale.code(), "changed");
  assert_eq!(std::fs::read_to_string(&real).unwrap(), "#!/bin/sh\necho hi\n");
  let exists = client.call("write", json!({ "path": path, "b": "", "ifAbsent": true })).await.unwrap_err();
  assert_eq!(exists.code(), "changed");
  // No temporary file is left next to it
  let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
  assert_eq!(names.len(), 1, "{names:?}");
}

#[tokio::test]
async fn large_reads_arrive_whole_through_the_window() {
  let dir = tempfile::tempdir().unwrap();
  let bytes: Vec<u8> = (0..3 * 1024 * 1024 + 17).map(|i| (i % 251) as u8).collect();
  std::fs::write(dir.path().join("big.bin"), &bytes).unwrap();
  let client = start(dir.path()).await;
  let (big, small) = tokio::join!(client.call("read", json!({ "path": format!("{ROOT}/big.bin") })), client.call("stat", json!({ "path": ROOT })));
  assert_eq!(big.unwrap().data, bytes);
  assert_eq!(small.unwrap().value["kind"], "dir");
}

#[tokio::test]
async fn exec_streams_output_takes_stdin_and_reports_the_exit() {
  let dir = tempfile::tempdir().unwrap();
  let client = start(dir.path()).await;
  let r = client.call("exec", json!({ "command": "pwd; echo err >&2; exit 3", "cwd": ROOT })).await.unwrap();
  assert_eq!(String::from_utf8_lossy(&r.out).trim(), dir.path().canonicalize().unwrap().to_string_lossy());
  assert_eq!(r.err, b"err\n");
  assert_eq!(r.value["code"], 3);
  assert_eq!(r.value["timedOut"], false);

  let mut call = client.request("exec", json!({ "argv": ["cat"], "stdin": true }));
  call.stdin(b"through stdin");
  call.stdin_eof();
  let mut got = vec![];
  loop {
    match call.next().await.unwrap() {
      Event::Chunk { bytes, .. } => got.extend(bytes),
      Event::Done(r) => {
        assert_eq!(r.unwrap()["code"], 0);
        break;
      }
    }
  }
  assert_eq!(got, b"through stdin");
}

/// The pid a command printed first
async fn first_line_pid(call: &mut acpira_host::remote::client::Call) -> i32 {
  let mut text = String::new();
  while !text.contains('\n') {
    match call.next().await.unwrap() {
      Event::Chunk { bytes, .. } => text.push_str(&String::from_utf8_lossy(&bytes)),
      Event::Done(r) => panic!("ended early: {r:?}"),
    }
  }
  text.lines().next().unwrap().trim().parse().unwrap()
}

#[tokio::test]
async fn a_timeout_or_a_cancel_kills_the_whole_group() {
  let dir = tempfile::tempdir().unwrap();
  let client = start(dir.path()).await;
  // A background child of the command dies with it when the timeout fires
  let mut call = client.request("exec", json!({ "command": "sleep 30 & echo $!; wait", "timeoutMs": 400 }));
  let child = first_line_pid(&mut call).await;
  let Some(Event::Done(r)) = call.next().await else { panic!("no end") };
  assert_eq!(r.unwrap()["timedOut"], true);
  assert!(gone_within(child, Duration::from_secs(3)).await, "background child {child} survived the timeout");

  let mut call = client.request("exec", json!({ "command": "sleep 30 & echo $!; wait" }));
  let child = first_line_pid(&mut call).await;
  drop(call);
  assert!(gone_within(child, Duration::from_secs(3)).await, "background child {child} survived the cancel");
  // The channel is still good for the next request
  assert!(client.call("stat", json!({ "path": ROOT })).await.is_ok());
}

#[tokio::test]
async fn closing_the_channel_ends_the_helper_and_what_it_started() {
  let dir = tempfile::tempdir().unwrap();
  let client = start(dir.path()).await;
  let helper = client.hello().pid as i32;
  let mut call = client.request("exec", json!({ "command": "echo $$; exec sleep 30" }));
  let child = first_line_pid(&mut call).await;
  client.close();
  let Some(Event::Done(Err(RemoteError::Disconnected(_)))) = call.next().await else { panic!("the running call should end as disconnected") };
  assert!(gone_within(helper, Duration::from_secs(5)).await, "helper {helper} still running");
  assert!(gone_within(child, Duration::from_secs(5)).await, "child {child} still running");
}

#[tokio::test]
async fn dropping_the_last_client_ends_the_helper() {
  let dir = tempfile::tempdir().unwrap();
  let client = start(dir.path()).await;
  let helper = client.hello().pid as i32;
  let copy = client.clone();
  drop(client);
  assert!(alive(helper), "a clone still holds the channel");
  drop(copy);
  assert!(gone_within(helper, Duration::from_secs(5)).await, "helper {helper} outlived its last client");
}

#[tokio::test]
async fn a_silent_client_is_given_up_with_its_processes() {
  // A client that never writes again (the laptop went to sleep): stdin stays open but nothing arrives
  let dir = tempfile::tempdir().unwrap();
  let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_acpira"));
  cmd.args(["remote", "--stdio", "--no-login-path", "--dead-after-ms", "800", "--ping-every-ms", "100"]);
  cmd.env("ACPIRA_REMOTE_ROOT_MAP", format!("{ROOT}={}", dir.path().display()));
  cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
  let mut child = cmd.spawn().unwrap();
  let mut stdin = child.stdin.take().unwrap();
  use std::io::{BufRead, Write};
  writeln!(stdin, r#"{{"t":"req","id":1,"op":"exec","a":{{"command":"echo $$; exec sleep 30"}}}}"#).unwrap();
  stdin.flush().unwrap();
  let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
  let mut pid = None;
  let mut line = String::new();
  while pid.is_none() && out.read_line(&mut line).unwrap() > 0 {
    if let Ok(acpira_host::remote::wire::Frame::Chunk { b, .. }) = serde_json::from_str(&line) {
      let text = String::from_utf8(acpira_host::remote::wire::unb64(&b).unwrap()).unwrap();
      pid = text.trim().parse::<i32>().ok();
    }
    line.clear();
  }
  let pid = pid.expect("the command's pid");
  let started = std::time::Instant::now();
  let status = child.wait().unwrap();
  assert_eq!(status.code(), Some(3), "ends with the heartbeat code");
  assert!(started.elapsed() < Duration::from_secs(5));
  assert!(gone_within(pid, Duration::from_secs(3)).await, "the command survived the helper");
  drop(stdin);
}

#[tokio::test]
async fn a_frozen_helper_is_given_up_and_its_requests_fail_as_unknown() {
  let dir = tempfile::tempdir().unwrap();
  let opts = ClientOpts { dead_after: Duration::from_millis(600), ping_every: Duration::from_millis(100), ..ClientOpts::default() };
  let client = RemoteClient::start(helper_cmd(dir.path(), &[]), opts).await.unwrap();
  let helper = client.hello().pid as i32;
  // SAFETY: stopping our own child
  unsafe { libc::kill(helper, libc::SIGSTOP) };
  let r = client.call("stat", json!({ "path": ROOT })).await;
  assert!(matches!(r, Err(RemoteError::Disconnected(ref why)) if why.contains("no answer")), "{r:?}");
  assert!(client.closed_reason().is_some());
  // SAFETY: the helper was killed on close; continuing a dead pid is harmless
  unsafe { libc::kill(helper, libc::SIGCONT) };
  assert!(gone_within(helper, Duration::from_secs(5)).await);
}
