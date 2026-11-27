//! Stopping running commands, the root's and a summoned subagent's

use super::*;

/// A stop of one running command ends that command's process alone: its row settles, the model gets the failed call
/// and answers, and the turn ends normally rather than cancelled
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn stopping_a_running_command_ends_it_and_the_turn_goes_on() {
  let env = Env::new();
  let s = env.session().await;
  s.set_config("approval".into(), "full".into()).await.unwrap();
  env.server.push(mock::tools(&[("c1", "bash", json!({ "command": "sleep 30 # stop me" }))]));
  env.server.push(mock::text("Stopped."));
  let task = spawn_prompt(&s, "go");
  // The row's id is the agent's own, not the model's call id
  let row = |s: &AcpSession| agent_blocks(&view(s)).into_iter().find(|b| b["kind"] == "execute");
  until(|| row(&s).is_some_and(|b| b["status"] == "in_progress"), 5000).await;
  assert_eq!(view(&s)["canStopCommands"], true);
  let id = row(&s).unwrap()["id"].as_str().unwrap().to_owned();
  let t0 = Instant::now();
  s.stop_command(&id, None).await.unwrap();
  until(|| task.is_finished(), 10_000).await;
  task.await.unwrap();
  assert!(t0.elapsed() < Duration::from_secs(10), "the command ran on");
  assert_ne!(row(&s).unwrap()["status"], "in_progress");
  let last = last_turn(&view(&s));
  assert_eq!(last["stop"], "end_turn", "{last}");
  assert!(agent_blocks(&view(&s)).iter().any(|b| b["type"] == "text" && b["markdown"] == "Stopped."));
  // The row has settled: a second stop finds nothing to end
  assert!(s.stop_command(&id, None).await.is_ok());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn stopping_a_summoned_subagents_command_ends_only_the_childs_command() {
  let env = Env::new();
  let s = env.session().await;
  s.set_config("approval".into(), "full".into()).await.unwrap();
  env.server.push(mock::tools(&[("p1", "bash", json!({ "command": "sleep 30 # parent keeps going" }))]));
  let parent = spawn_prompt(&s, "go parent");
  let parent_row = || agent_blocks(&view(&s)).into_iter().find(|b| b["kind"] == "execute");
  until(|| parent_row().is_some_and(|b| b["status"] == "in_progress"), 5000).await;
  assert_eq!(parent_row().unwrap()["id"], "call-1");

  let roster = Arc::new(Roster::new(env.root.path()));
  roster.save(&json!([{ "name": "Worker", "agent": "acpira", "mode": "bypass", "when": "work" }])).await.unwrap();
  env.server.push(mock::tools(&[("c1", "bash", json!({ "command": "sleep 30 # child stop me" }))]));
  env.server.push(mock::text("Child stopped."));
  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
  let ask = AskArgs { agent: "worker".into(), prompt: "run child".into(), wait: Some(true), wait_secs: Some(20), ..Default::default() };
  let child = tokio::spawn(s.0.clone().relay_ask(roster, None, 0, ask, tx));
  let node_id = || s.view().subagents.and_then(|nodes| nodes.first().map(|node| node.core.id.clone()));
  let child_row = |id: &str| {
    let (turns, _, _) = s.subagent_transcript(id)?;
    let turns: Value = serde_json::from_str(turns.0.get()).ok()?;
    agent_blocks(&json!({ "turns": turns })).into_iter().find(|b| b["kind"] == "execute")
  };
  until(|| node_id().is_some_and(|id| child_row(&id).is_some_and(|b| b["status"] == "in_progress")), 10_000).await;
  let id = node_id().unwrap();
  assert_eq!(child_row(&id).unwrap()["id"], "call-1");

  let start = Instant::now();
  s.stop_command("call-1", Some(&id)).await.unwrap();
  tokio::time::timeout(Duration::from_secs(10), child).await.expect("child did not finish").unwrap();
  let mut reply = None;
  while let Some(item) = rx.recv().await {
    if let HubReply::Done(text) = item {
      reply = Some(text);
    }
  }
  assert!(start.elapsed() < Duration::from_secs(10));
  assert!(reply.unwrap_or_default().contains("Child stopped."));
  assert_eq!(parent_row().unwrap()["status"], "in_progress");

  env.server.push(mock::text("Parent stopped."));
  s.stop_command("call-1", None).await.unwrap();
  tokio::time::timeout(Duration::from_secs(10), parent).await.expect("parent did not finish").unwrap();
  assert!(agent_blocks(&view(&s)).iter().any(|b| b["type"] == "text" && b["markdown"] == "Parent stopped."));
}
