//! Group calls: a round per enabled member, one message back, the running-round cap, the same pack

use super::*;

/// Two personas and a group of them (plus one disabled member, which a group call leaves out)
async fn group_roster(dir: &std::path::Path, group_name: &str) -> Arc<Roster> {
  let r = Arc::new(Roster::new(dir));
  r.save(&json!([
    { "name": "Alpha", "agent": "fake", "mode": "consult" },
    { "name": "Beta", "agent": "fake", "mode": "consult" },
    { "name": "Off", "agent": "fake", "mode": "consult", "enabled": false },
  ]))
  .await
  .unwrap();
  r.save(&json!({ "baseGroups": [], "groups": [{ "name": group_name, "members": ["alpha", "off", "beta"], "when": "two views" }] })).await.unwrap();
  r
}

#[tokio::test(flavor = "multi_thread")]
async fn a_group_call_gives_each_enabled_member_a_round_and_waits_for_all_their_replies() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = group_roster(h.dir.path(), "Pair").await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("pair", "relay: look")).await);
  assert!(ok, "{text}");
  assert!(text.starts_with("The replies of Pair (2 of its members):"), "{text}");
  let alpha = text.find("## Alpha (fake)\n\nreply to relay: look (round 1,").expect("alpha's reply");
  let beta = text.find("## Beta (fake)\n\nreply to relay: look (round 1,").expect("beta's reply");
  assert!(alpha < beta, "members in the group's order: {text}");
  // A round each, on threads of their own, marked with the group they came through
  let all = nodes(&s);
  assert_eq!(all.len(), 2, "{all:?}");
  assert!(all.iter().all(|n| n["harness"]["group"] == "pair"), "{all:?}");
  assert_ne!(all[0]["harness"]["thread"], all[1]["harness"]["thread"]);
  // A member's thread continues on its own
  let thread = all[1]["harness"]["thread"].as_str().unwrap().to_owned();
  let (_, again) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), ..ask("", "relay: and you?") }).await);
  assert!(again.starts_with("reply to relay: and you? (round 2,"), "{again}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_group_sent_off_from_the_root_comes_back_in_one_message_and_its_row_hides_behind_a_member() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  // `relay-call:` names fake-review: here the group's id
  let roster = group_roster(h.dir.path(), "Fake Review").await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  prompt(&s, "relay-call:relay: slow look").await;
  let (ok, text) = last_text(&run(&s, &roster, 0, send_off("fake-review", "relay: slow look")).await);
  assert!(ok && text.starts_with("Sent to the 2 members of Fake Review: Alpha (fake) (thread: "), "{text}");
  assert!(text.contains("Beta (fake) (thread: ") && text.contains("their replies together in one message"), "{text}");
  let first = nodes(&s)[0].clone();
  assert_eq!(first["peer"]["toolCallId"], "ak1");
  until(|| deliveries(&s).len() == 1 && !s.is_running(), 8000).await;
  let delivered = deliveries(&s)[0]["text"].as_str().unwrap().to_owned();
  assert!(delivered.contains("2 summoned agents") && delivered.contains("## Alpha (fake)") && delivered.contains("## Beta (fake)"), "{delivered}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_group_that_would_not_fit_beside_the_running_rounds_starts_none_of_them() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = group_roster(h.dir.path(), "Pair").await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  for i in 0..5 {
    let (ok, _) = last_text(&run(&s, &roster, 0, send_off("alpha", &format!("relay: slow {i}"))).await);
    assert!(ok);
  }
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("pair", "relay: look")).await);
  assert!(!ok && text.contains("Pair has 2 members, but 5 summoned agents are already running"), "{text}");
  assert_eq!(nodes(&s).len(), 5);
  until(|| !s.background_running(), 8000).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_member_of_a_group_gets_the_same_pack() {
  let fake = fake_or_skip!();
  let Some((_keep, root)) = repo() else { return };
  let h = Harness::new(&fake, json!({}));
  let roster = group_roster(h.dir.path(), "Pair").await;
  let s = Disposing(h.session(&root.to_string_lossy()));
  s.start().await;
  std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { changes: Some(true), ..ask("pair", "relay: echo") }).await);
  assert!(ok, "{text}");
  let packs: Vec<&str> = text.split("[resource ").skip(1).map(|p| p.split_once('\n').map(|x| x.1).unwrap_or("")).collect();
  assert_eq!(packs.len(), 2, "{text}");
  let body = |p: &str| p.split("\n---\n").next().unwrap_or("").to_owned();
  assert!(body(packs[0]).contains("+two") && body(packs[0]) == body(packs[1]), "{text}");
}
