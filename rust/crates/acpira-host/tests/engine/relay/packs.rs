//! The caller's changes handed to a thread: the changed files of a round, packs, scouts, executors and reviewers

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_round_reply_ends_with_the_files_it_changed_and_what_its_commands_changed_besides() {
  let fake = fake_or_skip!();
  let Some((_keep, root)) = repo() else { return };
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session(&root.to_string_lossy()));
  s.start().await;
  // The edit row is counted from its diff; the command wrote gen.txt, which only the work tree snapshot shows
  let task = format!("relay: edit={} run={}", root.join("a.txt").display(), root.join("gen.txt").display());
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", &task)).await);
  assert!(ok, "{text}");
  assert!(
    text.contains("(Acpira: files it edited: a.txt (+1 -0); these also changed while its commands ran: gen.txt.)"),
    "{text}"
  );
  // A work round that only read says so
  let (_, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", "relay: just look")).await);
  assert!(text.contains("(Acpira: it changed no files.)"), "{text}");
  // A read-only round gets a note only when it changed something anyway
  let (_, text) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: look")).await);
  assert!(!text.contains("(Acpira:"), "{text}");
  let task = format!("relay: run={}", root.join("other.txt").display());
  let (_, text) = last_text(&run(&s, &roster, 0, ask("fake-review", &task)).await);
  assert!(text.contains("(Acpira: it edited no file itself, but these changed while its commands ran: other.txt.)"), "{text}");
  // A round that failed after changing files still reports them with its failure
  let task = format!("relay: fail edit={}", root.join("a.txt").display());
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", &task)).await);
  assert!(!ok && text.contains("model overloaded") && text.contains("(Acpira: files it edited: a.txt"), "{text}");
}

fn git(root: &std::path::Path, args: &[&str]) {
  let out = std::process::Command::new("git").arg("-C").arg(root).args(args).output().unwrap();
  assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

async fn handing_roster(dir: &std::path::Path) -> Arc<Roster> {
  let r = Arc::new(Roster::new(dir));
  r.save(&json!([
    { "name": "Fake Review", "agent": "fake", "mode": "consult" },
    { "name": "Handed", "agent": "fake", "mode": "consult", "changes": true },
  ]))
  .await
  .unwrap();
  r
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_is_handed_the_callers_changes_once_and_then_hears_what_moved_since() {
  let fake = fake_or_skip!();
  let Some((_keep, root)) = repo() else { return };
  let h = Harness::new(&fake, json!({}));
  let roster = handing_roster(h.dir.path()).await;
  let s = Disposing(h.session(&root.to_string_lossy()));
  s.start().await;
  std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();

  // The first round: the brief says a pack is attached, and the pack is an embedded resource (the fake CLI takes them)
  let (ok, first) = last_text(&run(&s, &roster, 0, AskArgs { changes: Some(true), ..ask("fake-review", "relay: echo") }).await);
  assert!(ok, "{first}");
  assert!(first.contains("Attached is the handoff pack of the caller's uncommitted changes"), "{first}");
  assert!(first.contains("[resource file://") && first.contains("# Handoff: uncommitted changes") && first.contains("+two"), "{first}");
  assert!(!first.contains("(Acpira:"), "{first}");
  let thread = thread_of(&first);

  // A later round of the thread is told in one line what moved, without asking again; an untracked file counts
  std::fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
  std::fs::write(root.join("b.txt"), "new\n").unwrap();
  let (_, second) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), ..ask("", "relay: echo again") }).await);
  assert!(second.contains("Changed since your last round: a.txt (+2 -0 now), b.txt (newly changed, +1 -0)."), "{second}");
  assert!(!second.contains("[resource") && !second.contains("Attached is the handoff pack"), "{second}");
  // Nothing moved: no line at all
  let (_, third) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), ..ask("", "relay: echo once more") }).await);
  assert!(!third.contains("Changed since") && !third.contains("[resource"), "{third}");
  // Unless the call opts out
  std::fs::write(root.join("b.txt"), "new\nmore\n").unwrap();
  let (_, quiet) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), changes: Some(false), ..ask("", "relay: echo quiet") }).await);
  assert!(!quiet.contains("Changed since"), "{quiet}");

  // A persona whose user switched handover on gets the pack without asking, and a call can switch it off
  let (_, handed) = last_text(&run(&s, &roster, 0, ask("handed", "relay: echo")).await);
  assert!(handed.contains("[resource file://") && handed.contains("b.txt"), "{handed}");
  let (_, off) = last_text(&run(&s, &roster, 0, AskArgs { changes: Some(false), ..ask("handed", "relay: echo") }).await);
  assert!(!off.contains("[resource") && !off.contains("Attached is the handoff pack"), "{off}");
  // A plain call hands nothing over
  let (_, plain) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: echo")).await);
  assert!(!plain.contains("[resource") && !plain.contains("handoff"), "{plain}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pack_goes_inline_where_the_cli_takes_no_embedded_context_and_a_clean_tree_gets_a_note() {
  let fake = fake_or_skip!();
  let Some((_keep, root)) = repo() else { return };
  let h = Harness::new(&fake, json!({ "env": { "FAKE_PROMPT_CAPS": "strict" } }));
  let roster = handing_roster(h.dir.path()).await;
  let s = Disposing(h.session(&root.to_string_lossy()));
  s.start().await;
  std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("handed", "relay: echo")).await);
  assert!(ok, "{text}");
  assert!(text.contains("[Attachment: handoff-pack.md]\n# Handoff: uncommitted changes") && text.contains("[End of attachment: handoff-pack.md]"), "{text}");
  assert!(!text.contains("[resource"), "{text}");
  // Nothing uncommitted: the round runs on its brief alone and the caller is told why no pack came along
  git(&root, &["checkout", "--", "a.txt"]);
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("handed", "relay: echo")).await);
  assert!(ok && !text.contains("Attachment:"), "{text}");
  assert!(text.contains("(Acpira: there were no uncommitted changes to hand over.)"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pack_covers_the_files_the_caller_edited_until_a_round_it_summoned_could_write_anywhere() {
  let fake = fake_or_skip!();
  let Some((_keep, root)) = repo() else { return };
  let h = Harness::new(&fake, json!({}));
  // `relay-call:` names fake-review, here a persona that may edit
  let roster = Arc::new(Roster::new(h.dir.path()));
  roster
    .save(&json!([
      { "name": "Handed", "agent": "fake", "mode": "consult", "changes": true },
      { "name": "Fake Review", "agent": "fake", "mode": "work" },
    ]))
    .await
    .unwrap();
  let s = Disposing(h.session(&root.to_string_lossy()));
  s.start().await;
  // Someone else's change in the work tree, then the caller's own edit row
  std::fs::write(root.join("theirs.txt"), "not mine\n").unwrap();
  prompt(&s, &format!("hook-fix:{}", root.join("a.txt").display())).await;
  let (_, text) = last_text(&run(&s, &roster, 0, ask("handed", "relay: echo")).await);
  assert!(text.contains("a.txt") && text.contains("+fixed"), "{text}");
  assert!(!text.contains("theirs.txt"), "only the caller's own edits: {text}");
  // A round the caller summoned in work mode (its row tied to the node) may have written anywhere: from then on the
  // whole work tree goes along
  let task = format!("relay: run={}", root.join("gen.txt").display());
  prompt(&s, &format!("relay-call:{task}")).await;
  let (ok, ran) = last_text(&run(&s, &roster, 0, ask("fake-review", &task)).await);
  assert!(ok && ran.contains("gen.txt"), "{ran}");
  let (_, text) = last_text(&run(&s, &roster, 0, ask("handed", "relay: echo")).await);
  assert!(text.contains("theirs.txt") && text.contains("gen.txt") && text.contains("a.txt"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scout_or_an_executor_continues_its_idle_thread_and_a_reviewer_starts_fresh() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = Arc::new(Roster::new(h.dir.path()));
  roster
    .save(&json!([
      { "name": "Scout", "agent": "fake", "mode": "consult", "duty": "scout" },
      { "name": "Doer", "agent": "fake", "mode": "work", "duty": "executor" },
      { "name": "Checker", "agent": "fake", "mode": "consult", "duty": "reviewer" },
      { "name": "Plain", "agent": "fake", "mode": "consult" },
    ]))
    .await
    .unwrap();
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let call = |agent: &str, prompt: &str| ask(agent, prompt);

  // A scout called again without a thread continues the same native session
  let (_, first) = last_text(&run(&s, &roster, 0, call("scout", "relay: where is x")).await);
  let (_, second) = last_text(&run(&s, &roster, 0, call("scout", "relay: and y")).await);
  assert!(second.starts_with("reply to relay: and y (round 2, mode plan)"), "{second}");
  assert_eq!(thread_of(&first), thread_of(&second));
  // Unless the call asks for a fresh start
  let (_, fresh) = last_text(&run(&s, &roster, 0, AskArgs { new_thread: Some(true), ..call("scout", "relay: start over") }).await);
  assert!(fresh.starts_with("reply to relay: start over (round 1,"), "{fresh}");
  assert_ne!(thread_of(&fresh), thread_of(&first));
  // An executor too; the newest idle thread is the one continued
  let (_, a) = last_text(&run(&s, &roster, 0, call("doer", "relay: step one")).await);
  let (_, b) = last_text(&run(&s, &roster, 0, call("doer", "relay: step two")).await);
  assert!(b.starts_with("reply to relay: step two (round 2, mode agent)"), "{b}");
  assert_eq!(thread_of(&a), thread_of(&b));
  let (_, c) = last_text(&run(&s, &roster, 0, call("scout", "relay: once more")).await);
  assert_eq!(thread_of(&c), thread_of(&fresh), "{c}");
  // A reviewer and a general persona start fresh every time
  for agent in ["checker", "plain"] {
    let (_, one) = last_text(&run(&s, &roster, 0, call(agent, "relay: review")).await);
    let (_, two) = last_text(&run(&s, &roster, 0, call(agent, "relay: review again")).await);
    assert!(two.starts_with("reply to relay: review again (round 1,"), "{two}");
    assert_ne!(thread_of(&one), thread_of(&two));
  }

  // A busy thread is not continued: the new task gets a conversation of its own
  let (_, busy) = last_text(&run(&s, &roster, 0, AskArgs { wait_secs: Some(0), ..call("doer", "relay: slow job") }).await);
  assert!(busy.contains("is still working"), "{busy}");
  let (_, other) = last_text(&run(&s, &roster, 0, call("doer", "relay: meanwhile")).await);
  assert!(other.starts_with("reply to relay: meanwhile (round 1,"), "{other}");
  assert_ne!(thread_of(&busy), thread_of(&other));
  until(|| !s.background_running(), 8000).await;

  // Nor one whose process is gone (reopening it is not what the call asked for), and the call does not fall back to the
  // scout's older conversation either
  let (_, gone) = last_text(&run(&s, &roster, 0, call("scout", "relay: exit after this")).await);
  until(|| !s.background_running(), 4000).await;
  tokio::time::sleep(Duration::from_millis(300)).await;
  let (_, next) = last_text(&run(&s, &roster, 0, call("scout", "relay: after the exit")).await);
  assert!(next.starts_with("reply to relay: after the exit (round 1,"), "{next}");
  assert_ne!(thread_of(&gone), thread_of(&next));
}
