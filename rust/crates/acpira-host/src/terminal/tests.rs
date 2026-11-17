use super::*;

#[test]
fn utf8_stream_joins_split_characters_and_replaces_bad_bytes() {
  let mut s = Utf8Stream::default();
  let text = "终端 ok";
  let bytes = text.as_bytes();
  // Split inside the first character's three bytes
  assert_eq!(s.push(&bytes[..1]), "");
  assert_eq!(s.push(&bytes[1..4]), "终");
  assert_eq!(s.push(&bytes[4..]), "端 ok");
  assert_eq!(s.push(&[b'a', 0xff, b'b']), "a\u{FFFD}b");
  assert_eq!(s.push(&[0xe7, 0xbb]), "");
  assert_eq!(s.finish(), "\u{FFFD}");
  assert_eq!(s.finish(), "");
}

#[test]
fn keep_tail_trims_to_a_line_start() {
  let mut buf = String::new();
  keep_tail(&mut buf, "aaaa\nbbbb\n", 16);
  assert_eq!(buf, "aaaa\nbbbb\n");
  keep_tail(&mut buf, "cccc\ndddd\neeee\n", 16);
  // 25 bytes > 16 + 4: cut at 9 (inside "bbbb"), on to the next line start
  assert_eq!(buf, "cccc\ndddd\neeee\n");
  let mut wide = String::new();
  keep_tail(&mut wide, &"终".repeat(10), 8);
  assert!(wide.len() <= 10 && wide.chars().all(|c| c == '终'));
}

#[test]
fn program_names() {
  assert_eq!(program_name("/bin/zsh"), "zsh");
  assert_eq!(program_name("-zsh"), "zsh");
  assert_eq!(program_name(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"), "powershell");
}

#[cfg(unix)]
mod pty {
  use super::super::*;

  type Log = Arc<Mutex<Vec<HostMsg>>>;

  fn hub(script: &str) -> Arc<TerminalHub> {
    TerminalHub::with(Some(vec!["/bin/sh".into(), "-c".into(), script.into()]), Duration::from_millis(300), Duration::from_millis(50))
  }

  fn sink() -> (Log, Sink) {
    let log: Log = Default::default();
    let l = log.clone();
    (log, Arc::new(move |m| l.lock().push(m)))
  }

  fn output(log: &Log) -> String {
    log
      .lock()
      .iter()
      .map(|m| match m {
        HostMsg::TerminalOutput { data, .. } => data.as_str(),
        HostMsg::TerminalState { replay: Some(r), .. } => r.as_str(),
        _ => "",
      })
      .collect()
  }

  async fn until(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
      assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
      tokio::time::sleep(Duration::from_millis(10)).await;
    }
  }

  fn exit_of(log: &Log) -> Option<TerminalExit> {
    log.lock().iter().find_map(|m| match m {
      HostMsg::TerminalState { exit: Some(e), .. } => Some(e.clone()),
      _ => None,
    })
  }

  #[tokio::test(flavor = "multi_thread")]
  async fn input_reaches_the_shell_and_the_exit_follows_the_output() {
    let h = hub("printf ready; read line; printf 'got:%s' \"$line\"; exit 3");
    let (log, s) = sink();
    h.open(1, s, "t1", None, Some((100, 30)));
    assert!(matches!(&log.lock()[0], HostMsg::TerminalState { cols: Some(100), rows: Some(30), replay: Some(_), .. }));
    until("the prompt", || output(&log).contains("ready")).await;
    h.input("t1", "hello\r".into(), false);
    until("the exit", || exit_of(&log).is_some()).await;
    assert!(output(&log).contains("got:hello"));
    assert_eq!(exit_of(&log), Some(TerminalExit { code: 3, signal: None }));
    // The exit is posted after the output before it
    let last = log.lock().iter().rposition(|m| matches!(m, HostMsg::TerminalOutput { .. })).unwrap();
    let exit = log.lock().iter().position(|m| matches!(m, HostMsg::TerminalState { exit: Some(_), .. })).unwrap();
    assert!(exit > last);
  }

  #[tokio::test(flavor = "multi_thread")]
  async fn a_page_coming_back_gets_the_output_so_far_and_the_size() {
    let h = hub("printf 'line one\\n'; stty size; read x; stty size; sleep 5");
    let (log, s) = sink();
    h.open(1, s, "t2", None, Some((90, 20)));
    until("the size", || output(&log).contains("20 90")).await;
    h.close(1, "t2");
    let (again, s2) = sink();
    h.open(2, s2, "t2", None, None);
    match &again.lock()[0] {
      HostMsg::TerminalState { replay: Some(r), cols, rows, title, .. } => {
        assert!(r.contains("line one") && r.contains("20 90"), "{r:?}");
        assert_eq!((*cols, *rows), (Some(90), Some(20)));
        assert!(title.is_some());
      }
      m => panic!("{m:?}"),
    }
    h.resize("t2", 120, 40);
    h.input("t2", "\r".into(), false);
    until("the new size", || output(&again).contains("40 120")).await;
    // The page that left heard nothing after it left
    assert!(!output(&log).contains("40 120"));
    h.kill("t2");
  }

  #[tokio::test(flavor = "multi_thread")]
  async fn output_pauses_for_a_page_that_does_not_keep_up() {
    let h = hub("i=0; while [ $i -lt 4000 ]; do echo 0123456789012345678901234567890123456789012345678901234567890123456789; i=$((i+1)); done; echo done");
    let (log, s) = sink();
    h.open(1, s, "t3", None, None);
    until("a pause", || h.get("t3").is_some_and(|t| t.state.lock().paused.is_some())).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let units = |log: &Log| -> u64 {
      log.lock().iter().map(|m| if let HostMsg::TerminalOutput { data, .. } = m { data.encode_utf16().count() as u64 } else { 0 }).sum()
    };
    let held = units(&log);
    assert!(held > HIGH_WATER && held < HIGH_WATER + 2 * BATCH_MAX as u64, "{held}");
    assert!(!output(&log).contains("done"));
    // Acknowledging all of it lets the rest through
    let mut acked = 0;
    until("the rest", || {
      let now = units(&log);
      if now > acked {
        h.ack(1, "t3", now - acked);
        acked = now;
      }
      output(&log).contains("done")
    })
    .await;
    h.kill("t3");
  }

  #[tokio::test(flavor = "multi_thread")]
  async fn kill_ends_the_shell_and_an_unviewed_terminal_ends_on_its_own() {
    let h = hub("echo pid:$$; sleep 30");
    let (log, s) = sink();
    h.open(1, s, "t4", None, None);
    until("the pid", || output(&log).contains("pid:")).await;
    let text = output(&log);
    let pid: i32 = text.split("pid:").nth(1).unwrap().split_whitespace().next().unwrap().parse().unwrap();
    h.kill("t4");
    assert!(h.get("t4").is_none());
    // SAFETY: signal 0 only checks that the process exists
    until("the shell to end", || unsafe { libc::kill(pid, 0) } != 0).await;
    let n = log.lock().len();
    h.input("t4", "x".into(), false);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(log.lock().len(), n);

    let (_log, s) = sink();
    h.open(7, s, "t5", None, None);
    h.detach_view(7);
    until("the orphan to end", || h.get("t5").is_none()).await;
  }

  #[tokio::test(flavor = "multi_thread")]
  async fn the_title_follows_the_foreground_process() {
    let h = TerminalHub::with(Some(vec!["/bin/sh".into(), "-c".into(), "exec sleep 5".into()]), Duration::from_secs(60), Duration::from_millis(50));
    let (log, s) = sink();
    h.open(1, s, "t6", None, None);
    assert!(matches!(&log.lock()[0], HostMsg::TerminalState { title: Some(t), .. } if t == "sh"));
    if cfg!(any(target_os = "macos", target_os = "linux")) {
      until("the title", || log.lock().iter().any(|m| matches!(m, HostMsg::TerminalState { title: Some(t), replay: None, .. } if t == "sleep"))).await;
    }
    h.dispose();
    assert!(h.get("t6").is_none());
  }

  #[tokio::test(flavor = "multi_thread")]
  async fn an_invalid_starting_folder_is_reported_without_starting_a_shell() {
    let h = hub("echo must-not-start");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file");
    std::fs::write(&file, "file").unwrap();
    for cwd in [
      dir.path().join("missing").to_string_lossy().into_owned(),
      file.to_string_lossy().into_owned(),
      ".".into(),
      "".into(),
      "ssh://host/project/../elsewhere".into(),
      "ssh://-option/project".into(),
    ] {
      let (log, s) = sink();
      h.open(1, s, "invalid", Some(&cwd), None);
      assert!(matches!(&log.lock()[0], HostMsg::TerminalState { error: Some(_), .. }), "cwd: {cwd}");
      assert!(h.get("invalid").is_none(), "cwd: {cwd}");
    }
  }

  #[tokio::test(flavor = "multi_thread")]
  async fn a_shell_that_cannot_start_says_why() {
    let h = TerminalHub::with(Some(vec!["/nonexistent/acpira-shell".into()]), Duration::from_secs(60), Duration::from_millis(50));
    let (log, s) = sink();
    h.open(1, s, "t7", None, None);
    assert!(matches!(&log.lock()[0], HostMsg::TerminalState { error: Some(_), .. }));
    assert!(h.get("t7").is_none());
  }
}
