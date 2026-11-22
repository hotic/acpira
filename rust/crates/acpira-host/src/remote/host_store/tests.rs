use super::*;

fn body_of(bytes: Vec<u8>) -> Option<mpsc::UnboundedReceiver<Option<Vec<u8>>>> {
  let (tx, rx) = mpsc::unbounded_channel();
  tx.send(Some(bytes)).unwrap();
  tx.send(None).unwrap();
  Some(rx)
}

#[tokio::test]
async fn owned_records_and_native_logs_refuse_later_foreign_writes() {
  let dir = tempfile::tempdir().unwrap();
  let store = TranscriptStore::new(dir.path().join("sessions"), Arc::new(|_| {}), None);
  let mut record: SessionRecord = serde_json::from_value(json!({
    "id": "session", "agent": "acpira", "acpSessionId": "native", "cwd": "/work", "title": "owned",
    "updatedAt": "2026-01-01T00:00:00.000Z", "turns": []
  })).unwrap();
  store.flush(Arc::new(record.clone())).await.unwrap();
  record.copy_of = Some(crate::store::record::CopyOf { machine: "source".into(), name: "Source".into() });
  record.updated_at = "2999-01-01T00:00:00.000Z".into();
  let args = json!({ "home": dir.path(), "sessionId": "session", "id": "native" });
  let result = serve("storePut", &args, body_of(gzip(&serde_json::to_vec(&record).unwrap()).unwrap())).await.unwrap();
  assert_eq!(result["written"], false);
  assert_eq!(result["takenOver"], true);
  assert!(store.load("session").await.unwrap().copy_of.is_none());
  let result = serve("storeAgentLog", &args, body_of(gzip(b"do not replace").unwrap())).await.unwrap();
  assert_eq!(result["written"], false);
  assert!(!dir.path().join("agent/sessions/native.jsonl").exists());
  let bad = json!({ "home": dir.path(), "sessionId": "session", "id": "../outside" });
  assert!(serve("storeAgentLog", &bad, body_of(gzip(b"bad").unwrap())).await.is_err());
  let bytes = get_record(&json!({ "home": dir.path(), "id": "session" })).await.unwrap();
  assert!(serde_json::from_slice::<SessionRecord>(&gunzip(&bytes).unwrap()).unwrap().copy_of.is_none());
}

#[tokio::test]
async fn a_new_copy_drops_the_native_log_of_the_one_it_replaces() {
  let dir = tempfile::tempdir().unwrap();
  let mut record: SessionRecord = serde_json::from_value(json!({
    "id": "session", "agent": "acpira", "acpSessionId": "native", "cwd": "/work", "title": "copied",
    "updatedAt": "2026-01-01T00:00:00.000Z", "turns": [], "copyOf": { "machine": "source", "name": "Source" }
  })).unwrap();
  let args = json!({ "home": dir.path(), "sessionId": "session", "id": "native" });
  let log = dir.path().join("agent/sessions/native.jsonl");
  let put = serve("storePut", &args, body_of(gzip(&serde_json::to_vec(&record).unwrap()).unwrap())).await.unwrap();
  assert_eq!(put["written"], true);
  assert_eq!(serve("storeAgentLog", &args, body_of(gzip(b"first").unwrap())).await.unwrap()["written"], true);
  assert_eq!(std::fs::read(&log).unwrap(), b"first");
  // The next copy lands and its log does not (yet): a promotion now must not resume the context of the copy before
  record.updated_at = "2026-01-02T00:00:00.000Z".into();
  let put = serve("storePut", &args, body_of(gzip(&serde_json::to_vec(&record).unwrap()).unwrap())).await.unwrap();
  assert_eq!(put["written"], true);
  assert!(!log.exists());
  assert_eq!(serve("storeAgentLog", &args, body_of(gzip(b"second").unwrap())).await.unwrap()["written"], true);
  assert_eq!(std::fs::read(&log).unwrap(), b"second");
}

#[test]
fn a_subagent_ahead_of_the_host_is_local_work_to_keep() {
  let host: SessionRecord = serde_json::from_value(json!({
    "id": "session", "agent": "claude", "cwd": "/work", "updatedAt": "1", "turns": [{ "role": "user", "text": "go" }],
    "subagents": [{ "id": "child", "turnIndex": 0, "visibility": "session", "state": "running", "stateSource": "agent", "announcedAt": 1,
      "turns": [{ "role": "user", "text": "task" }] }]
  })).unwrap();
  let mut local = host.clone();
  assert!(is_prefix(&local, &host));
  local.subagents.as_mut().unwrap()[0].turns.push(serde_json::from_value(json!({ "role": "user", "text": "written after the last copy" })).unwrap());
  assert!(!is_prefix(&local, &host), "the session's own turns match, the subagent's do not");
  assert!(is_prefix(&host, &local));
}

#[tokio::test]
async fn native_logs_append_incrementally_and_fall_back_on_rewind_or_interruption() {
  let dir = tempfile::tempdir().unwrap();
  let local_root = dir.path().join("local");
  let host_root = dir.path().join("host");
  std::fs::create_dir_all(local_root.join("agent/sessions")).unwrap();
  let local_log = local_root.join("agent/sessions/native.jsonl");
  let host_log = host_root.join("agent/sessions/native.jsonl");
  let staged_log = host_root.join("agent/sessions/native/.sync.jsonl");

  let mut record: SessionRecord = serde_json::from_value(json!({
    "id": "session", "agent": "acpira", "acpSessionId": "native", "cwd": "/work", "title": "copied",
    "updatedAt": "2026-01-01T00:00:00.000Z", "turns": [], "copyOf": { "machine": "source", "name": "Source" }
  }))
  .unwrap();

  // Start with a multi-window (> 2 * LOG_EDGE) JSONL log so head and tail stamps cover distinct regions
  let mut expected = Vec::new();
  for i in 0..800 {
    expected.extend_from_slice(format!("{{\"type\":\"item\",\"t\":{i},\"text\":\"line-{i:04}-padding-for-realistic-log-volume\"}}\n").as_bytes());
  }
  assert!(expected.len() as u64 > 4 * LOG_EDGE);
  std::fs::write(&local_log, &expected).unwrap();

  // 1. Initial sync: no parked base on the host yet, so the whole log goes
  let put = serve("storePut", &json!({ "home": host_root }), body_of(gzip(&serde_json::to_vec(&record).unwrap()).unwrap())).await.unwrap();
  assert_eq!(put["written"], true);
  assert!(put.get("agentLog").is_none());
  let prep = prepare_log_upload(local_log.clone(), None).await.unwrap().unwrap();
  assert!(prep.base.is_none());
  let full_gz_len = prep.gz.len();
  let res = serve("storeAgentLog", &json!({ "home": host_root, "sessionId": "session", "id": "native" }), body_of(prep.gz)).await.unwrap();
  assert_eq!(res["written"], true);
  assert_eq!(std::fs::read(&host_log).unwrap(), expected);

  // 2. Append one new step locally: `storePut` parks `native.jsonl` (promotion guard holds) and `storeAgentLog`
  //    carries only the newly appended bytes
  let delta = b"{\"type\":\"item\",\"t\":801,\"text\":\"one small incremental step\"}\n";
  expected.extend_from_slice(delta);
  std::fs::write(&local_log, &expected).unwrap();
  record.updated_at = "2026-01-02T00:00:00.000Z".into();
  let put = serve("storePut", &json!({ "home": host_root }), body_of(gzip(&serde_json::to_vec(&record).unwrap()).unwrap())).await.unwrap();
  assert_eq!(put["written"], true);
  assert!(!host_log.exists(), "stale live log must not stay visible before storeAgentLog lands");
  assert!(staged_log.exists());
  let stamp: LogStamp = serde_json::from_value(put["agentLog"].clone()).unwrap();
  let prep = prepare_log_upload(local_log.clone(), Some(stamp)).await.unwrap().unwrap();
  assert!(prep.base.is_some(), "pure append should match the host's parked prefix");
  assert!(prep.gz.len() < 200 && prep.gz.len() * 10 < full_gz_len, "delta gz={} vs full gz={full_gz_len}", prep.gz.len());
  assert_eq!(gunzip(&prep.gz).unwrap(), delta);
  let base = prep.base.unwrap();
  let res = serve(
    "storeAgentLog",
    &json!({ "home": host_root, "sessionId": "session", "id": "native", "offset": base.size, "digest": base.digest }),
    body_of(prep.gz),
  )
  .await
  .unwrap();
  assert_eq!(res["written"], true);
  assert_eq!(std::fs::read(&host_log).unwrap(), expected);
  assert!(!staged_log.exists());

  // 3. Record metadata edit with 0 new log bytes: unparks in place with an empty delta
  record.title = "renamed".into();
  record.updated_at = "2026-01-03T00:00:00.000Z".into();
  let put = serve("storePut", &json!({ "home": host_root }), body_of(gzip(&serde_json::to_vec(&record).unwrap()).unwrap())).await.unwrap();
  let stamp: LogStamp = serde_json::from_value(put["agentLog"].clone()).unwrap();
  let prep = prepare_log_upload(local_log.clone(), Some(stamp)).await.unwrap().unwrap();
  assert!(prep.base.is_some());
  assert!(gunzip(&prep.gz).unwrap().is_empty());
  let base = prep.base.unwrap();
  let res = serve(
    "storeAgentLog",
    &json!({ "home": host_root, "sessionId": "session", "id": "native", "offset": base.size, "digest": base.digest }),
    body_of(prep.gz),
  )
  .await
  .unwrap();
  assert_eq!(res["written"], true);
  assert_eq!(std::fs::read(&host_log).unwrap(), expected);

  // 4. Interrupted sync: `storePut` parks the log, connection drops before `storeAgentLog`, then another `storePut`
  //    arrives after more local appends — the parked base still survives for an incremental delta
  record.updated_at = "2026-01-04T00:00:00.000Z".into();
  let put1 = serve("storePut", &json!({ "home": host_root }), body_of(gzip(&serde_json::to_vec(&record).unwrap()).unwrap())).await.unwrap();
  assert!(put1.get("agentLog").is_some());
  assert!(!host_log.exists() && staged_log.exists());
  let more = b"{\"type\":\"item\",\"t\":802,\"text\":\"after reconnect\"}\n";
  expected.extend_from_slice(more);
  std::fs::write(&local_log, &expected).unwrap();
  record.updated_at = "2026-01-05T00:00:00.000Z".into();
  let put2 = serve("storePut", &json!({ "home": host_root }), body_of(gzip(&serde_json::to_vec(&record).unwrap()).unwrap())).await.unwrap();
  let stamp: LogStamp = serde_json::from_value(put2["agentLog"].clone()).unwrap();
  let prep = prepare_log_upload(local_log.clone(), Some(stamp)).await.unwrap().unwrap();
  assert_eq!(gunzip(&prep.gz).unwrap(), more);
  let base = prep.base.unwrap();
  let res = serve(
    "storeAgentLog",
    &json!({ "home": host_root, "sessionId": "session", "id": "native", "offset": base.size, "digest": base.digest }),
    body_of(prep.gz),
  )
  .await
  .unwrap();
  assert_eq!(res["written"], true);
  assert_eq!(std::fs::read(&host_log).unwrap(), expected);

  // 5. Rewind / prefix rewrite locally (even when longer than before): falls back to full upload and cleans `.sync.jsonl`
  let mut rewound = b"{\"type\":\"session\",\"t\":999,\"rewound\":true}\n".to_vec();
  rewound.resize(expected.len() + 64, b'\n');
  std::fs::write(&local_log, &rewound).unwrap();
  record.updated_at = "2026-01-06T00:00:00.000Z".into();
  let put = serve("storePut", &json!({ "home": host_root }), body_of(gzip(&serde_json::to_vec(&record).unwrap()).unwrap())).await.unwrap();
  let stamp: LogStamp = serde_json::from_value(put["agentLog"].clone()).unwrap();
  let prep = prepare_log_upload(local_log.clone(), Some(stamp)).await.unwrap().unwrap();
  assert!(prep.base.is_none(), "rewound prefix must trigger full replacement");
  // Also verify a stale offset request on the host asks for `resync` without corrupting anything
  let stale_try = serve(
    "storeAgentLog",
    &json!({ "home": host_root, "sessionId": "session", "id": "native", "offset": 123, "digest": "wrong" }),
    body_of(gzip(b"x").unwrap()),
  )
  .await
  .unwrap();
  assert_eq!((stale_try["written"].as_bool(), stale_try["resync"].as_bool()), (Some(false), Some(true)));
  let res = serve("storeAgentLog", &json!({ "home": host_root, "sessionId": "session", "id": "native" }), body_of(prep.gz)).await.unwrap();
  assert_eq!(res["written"], true);
  assert_eq!(std::fs::read(&host_log).unwrap(), rewound);
  assert!(!staged_log.exists());
}

#[tokio::test]
async fn schedule_waits_for_an_active_flush_instead_of_overlapping() {
  let dir = tempfile::tempdir().unwrap();
  let root = dir.path();
  let connections = Connections::new(root, root, Arc::new(|_| {}), None, Default::default());
  let store = TranscriptStore::new(root.join("sessions"), Arc::new(|_| {}), None);
  let mirror = HostMirror::with_batch(connections, store, root, Arc::new(|_| {}), Duration::from_millis(20));

  // Simulate an in-flight flush: rapid schedules during it must coalesce into `again` without starting a batch timer
  mirror.state.lock().flushing = true;
  mirror.schedule();
  mirror.schedule();
  {
    let st = mirror.state.lock();
    assert!(!st.batching, "must not start a parallel batch timer while a flush is running");
    assert!(st.again, "must remember to schedule once the running flush finishes");
  }
  mirror.state.lock().flushing = false;
}

#[tokio::test]
async fn push_deduplicates_unchanged_copies_and_sends_only_log_deltas_over_the_wire() {
  use std::sync::atomic::Ordering;
  let dir = tempfile::tempdir().unwrap();
  let local_root = dir.path().join("local");
  let host_root = dir.path().join("host");
  std::fs::create_dir_all(local_root.join("agent/sessions")).unwrap();
  let connections = Connections::new(&local_root, &local_root, Arc::new(|_| {}), None, Default::default());
  let store = TranscriptStore::new(local_root.join("sessions"), Arc::new(|_| {}), None);
  let mirror = HostMirror::with_batch(connections, store.clone(), &local_root, Arc::new(|_| {}), Duration::from_secs(60));

  let local_log = local_root.join("agent/sessions/native-wire.jsonl");
  let host_log = host_root.join("agent/sessions/native-wire.jsonl");
  let mut log_bytes = Vec::new();
  for i in 0..2500 {
    // Pseudo-unique payloads so the initial log compresses to many KiB on the wire
    let noise = crate::store::transcript_store::blob_name("", format!("seed-{i}").as_bytes());
    log_bytes.extend_from_slice(format!("{{\"type\":\"item\",\"t\":{i},\"h\":\"{noise}\"}}\n").as_bytes());
  }
  std::fs::write(&local_log, &log_bytes).unwrap();
  let mut rec: SessionRecord = serde_json::from_value(json!({
    "id": "s-wire", "agent": "acpira", "acpSessionId": "native-wire", "cwd": "ssh://example-host/work/proj",
    "title": "wire", "category": Some("cat-a"), "createdAt": "2026-10-01T00:00:00.000Z",
    "updatedAt": "2026-10-01T00:00:00.000Z", "turns": [], "controls": { "modes": [], "options": [] }, "commands": []
  }))
  .unwrap();
  store.flush(Arc::new(rec.clone())).await.unwrap();

  let (client, uplink) = super::super::client::test_channel(64 * 1024, Duration::ZERO).await;

  // 1. First push sends the whole ~130 KiB log
  uplink.store(0, Ordering::Relaxed);
  mirror.push_to(&client, "example-host", "s-wire", Some(&host_root)).await.unwrap();
  let first_bytes = uplink.load(Ordering::Relaxed);
  assert!(first_bytes > 20_000, "initial push should carry the full compressed log ({first_bytes} B)");
  assert_eq!(std::fs::read(&host_log).unwrap(), log_bytes);

  // 2. Duplicate push and local-only `category` move send 0 wire bytes
  uplink.store(0, Ordering::Relaxed);
  mirror.push_to(&client, "example-host", "s-wire", Some(&host_root)).await.unwrap();
  rec.category = Some("cat-b".into());
  store.flush(Arc::new(rec.clone())).await.unwrap();
  mirror.push_to(&client, "example-host", "s-wire", Some(&host_root)).await.unwrap();
  assert_eq!(uplink.load(Ordering::Relaxed), 0, "unchanged host copy must not re-upload");

  // 3. One appended step sends only the small record + tiny log delta (a fraction of `first_bytes`)
  log_bytes.extend_from_slice(b"{\"type\":\"item\",\"t\":9999,\"h\":\"step-delta\"}\n");
  std::fs::write(&local_log, &log_bytes).unwrap();
  rec.updated_at = "2026-10-01T00:00:05.000Z".into();
  store.flush(Arc::new(rec.clone())).await.unwrap();
  uplink.store(0, Ordering::Relaxed);
  mirror.push_to(&client, "example-host", "s-wire", Some(&host_root)).await.unwrap();
  let step_bytes = uplink.load(Ordering::Relaxed);
  assert!(step_bytes < 2_500 && step_bytes * 10 < first_bytes, "incremental step sent {step_bytes} B vs initial {first_bytes} B");
  assert_eq!(std::fs::read(&host_log).unwrap(), log_bytes);

  client.close();
}
