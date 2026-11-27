//! The engine's own MCP server: show_image, show_video and read_session reaching the model and the transcript

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn the_engines_own_mcp_server_reaches_the_model_and_show_image_shows_the_picture() {
  let mut env = Env::new();
  // What the runtime hands every session: `acpira mcp` on stdio (show_image, workspace_hooks)
  env.deps.host_mcp = Some(std::sync::Arc::new(acpira_host::host_mcp::HostMcp::new(env!("CARGO_BIN_EXE_acpira"))));
  let png = env.cwd().join("shot.png");
  std::fs::write(&png, b"\x89PNG\r\n\x1a\n").unwrap();
  let s = env.session().await;
  env.server.push(mock::tools(&[("c1", "acpira_show_image", json!({ "path": png.to_string_lossy() }))]));
  env.server.push(mock::text("Here it is."));
  let cards = run(&s, "show me the shot").await;
  assert!(cards.is_empty(), "the engine's own tools ask nothing: {cards:?}");
  let reqs = env.server.requests();
  let names: Vec<String> = reqs[0].body["tools"].as_array().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap().to_owned()).collect();
  assert!(names.iter().any(|n| n == "acpira_show_image") && names.iter().any(|n| n == "acpira_workspace_hooks"), "{names:?}");
  // The server's instructions came with the first message, the receipt with the result
  let first = reqs[0].body["messages"][1]["content"].as_str().unwrap().to_owned();
  assert!(first.starts_with("<mcp>") && first.contains("## acpira\n"), "{first}");
  let msgs = reqs[1].body["messages"].as_array().unwrap();
  let receipt = msgs.iter().find(|m| m["role"] == "tool").unwrap()["content"].as_str().unwrap().to_owned();
  assert!(receipt.contains("shot.png"), "{receipt}");
  // The engine knows the call by its name: a show-image row carrying the picture
  let vw = view(&s);
  let row = agent_blocks(&vw).into_iter().find(|b| b["type"] == "tool_call").unwrap();
  expect_match(&row, json!({ "verbKey": "verb.showImage", "status": "completed" }));
  assert!(row.to_string().contains("image/png"), "the picture is attached: {row}");
  env.assert_prefix_stable(&env.log_of(&s));
}

#[tokio::test(flavor = "multi_thread")]
async fn show_video_saves_the_recording_without_sending_it_back_to_the_model() {
  let mut env = Env::new();
  env.deps.host_mcp = Some(std::sync::Arc::new(acpira_host::host_mcp::HostMcp::new(env!("CARGO_BIN_EXE_acpira"))));
  let clip = env.cwd().join("clip.mp4");
  let bytes = b"\0\0\0\x18ftypmp42 recording";
  std::fs::write(&clip, bytes).unwrap();
  let s = env.session().await;
  env.server.push(mock::tools(&[("v", "acpira_show_video", json!({ "path": clip, "caption": "Recording" }))]));
  env.server.push(mock::text("Shown."));
  assert!(run(&s, "show the recording").await.is_empty());
  let vw = view(&s);
  let row = agent_blocks(&vw).into_iter().find(|b| b["verbKey"] == "verb.showVideo").unwrap();
  expect_match(&row, json!({ "status": "completed", "target": "Recording", "content": { "type": "image", "mimeType": "video/mp4" } }));
  let path = env.root.path().join("sessions").join(&s.id).join(row["content"]["blob"].as_str().unwrap());
  until(|| path.exists(), 5000).await;
  assert_eq!(std::fs::read(path).unwrap(), bytes);
  let sent = env.server.requests().last().unwrap().body["messages"].to_string();
  assert!(sent.contains("paused video player") && !sent.contains("ftypmp42"), "{sent}");
}

/// Stands in for the session manager behind the hub: what reaches it, and a fixed answer
struct CannedLibrary;

impl acpira_host::session_library::SessionLibrary for CannedLibrary {
  fn search_sessions<'a>(
    &'a self,
    _caller: &'a acpira_host::session_library::LibraryCaller,
    args: acpira_host::relay::wire::SessionSearchArgs,
  ) -> acpira_host::session_library::LibraryFuture<'a> {
    Box::pin(async move { Ok(format!("searched {:?}", args.query)) })
  }
  fn read_session<'a>(
    &'a self,
    caller: &'a acpira_host::session_library::LibraryCaller,
    args: acpira_host::relay::wire::SessionReadArgs,
  ) -> acpira_host::session_library::LibraryFuture<'a> {
    let me = caller.session_id.clone();
    Box::pin(async move { Ok(format!("# Outline of {} 2-3 asked by {me}", args.session)) })
  }
}

// The whole way an agent reads another session: the model's call, the built-in agent's MCP client, the `acpira mcp` process,
// the hub with the session's grant, the library, and back into a "Read session" row naming the turns
#[tokio::test(flavor = "multi_thread")]
async fn read_session_goes_from_the_model_through_acpira_mcp_and_the_hub_into_a_session_row() {
  let mut env = Env::new();
  let roster = Arc::new(acpira_host::relay::roster::Roster::new(env.root.path()));
  let hub = acpira_host::relay::hub::RelayHub::start(roster, Arc::new(|_: &str| {})).await.unwrap();
  let library: Arc<dyn acpira_host::session_library::SessionLibrary> = Arc::new(CannedLibrary);
  hub.set_library(Arc::downgrade(&library));
  env.deps.host_mcp = Some(std::sync::Arc::new(acpira_host::host_mcp::HostMcp::new(env!("CARGO_BIN_EXE_acpira")).with_hub(hub)));
  let s = env.session().await;
  let args = json!({ "session": "vscode://hotic.acpira/session/abc", "from": 2, "to": 3 });
  env.server.push(mock::tools(&[("r1", "acpira_read_session", args)]));
  env.server.push(mock::text("Read it."));
  let cards = run(&s, "what happened in that session?").await;
  assert!(cards.is_empty(), "a read-only engine tool asks nothing: {cards:?}");
  let reqs = env.server.requests();
  let names: Vec<String> = reqs[0].body["tools"].as_array().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap().to_owned()).collect();
  assert!(names.iter().any(|n| n == "acpira_search_sessions") && names.iter().any(|n| n == "acpira_read_session"), "{names:?}");
  let first = reqs[0].body["messages"][1]["content"].as_str().unwrap().to_owned();
  assert!(first.contains("acpira://session/<id>"), "the instructions name the links: {first}");
  let msgs = reqs[1].body["messages"].as_array().unwrap();
  let result = msgs.iter().find(|m| m["role"] == "tool").unwrap()["content"].as_str().unwrap().to_owned();
  assert!(result.contains(&format!("# Outline of vscode://hotic.acpira/session/abc 2-3 asked by {}", s.0.id)), "{result}");
  let vw = view(&s);
  let row = agent_blocks(&vw).into_iter().find(|b| b["type"] == "tool_call").unwrap();
  expect_match(&row, json!({ "verbKey": "verb.readSession", "target": "acpira://session/abc 2-3", "status": "completed" }));
}
