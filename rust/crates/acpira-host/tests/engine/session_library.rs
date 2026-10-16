//! Agents reading other sessions (`session_library.rs`, `search_sessions` / `read_session`): a real manager with fake-agent
//! sessions in two projects, read through the loopback hub with a root session's grant and a summoned thread's

use std::sync::{Arc, Weak};

use serde_json::json;

use acpira_host::relay::hub::{RelayHub, call};
use acpira_host::relay::roster::Roster;
use acpira_host::relay::wire::{HubOp, HubReply, HubRequest, SessionReadArgs, SessionSearchArgs};
use acpira_host::session_library::{SELF_READ, SessionLibrary};

use crate::fake_or_skip;
use crate::session_manager::{Mgr, Opts};
use crate::support::until;

/// The final line of one hub call: Ok for Done, Err for Error, None when the hub did not answer
async fn op(addr: &str, token: &str, op: HubOp) -> Option<Result<String, String>> {
  let (addr, req) = (addr.to_owned(), HubRequest { token: token.into(), op });
  tokio::task::spawn_blocking(move || {
    let mut end = None;
    call(&addr, &req, |r| {
      end = match r {
        HubReply::Done(t) => Some(Ok(t)),
        HubReply::Error(t) => Some(Err(t)),
        _ => return true,
      };
      false
    })
    .unwrap();
    end
  })
  .await
  .unwrap()
}

fn search(query: &str) -> HubOp {
  HubOp::SearchSessions(SessionSearchArgs { query: Some(query.into()), ..Default::default() })
}

fn read(session: &str) -> HubOp {
  HubOp::ReadSession(SessionReadArgs { session: session.into(), ..Default::default() })
}

#[tokio::test(flavor = "multi_thread")]
async fn agents_search_and_read_sessions_through_the_hub_within_their_grant() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let (proj_a, proj_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
  let opts = Opts::fake(&fake).cwd(proj_a.path().to_str().unwrap());
  let cwd = opts.cwd.clone();
  let m = Mgr::new(dir.path(), opts);
  m.init().await;
  // A and B in project A, C in project B; B is the caller
  let mut ids = vec![];
  for (text, project) in [("zebracorn alpha question", &proj_a), ("beta question", &proj_a), ("zebracorn gamma question", &proj_b)] {
    *cwd.lock().unwrap() = project.path().to_str().unwrap().into();
    m.new_session(None).await;
    m.handle(json!({ "type": "send", "text": text })).await;
    ids.push(m.active_id().unwrap());
  }
  let (a, b, c) = (ids[0].clone(), ids[1].clone(), ids[2].clone());
  // The conversation search reads the records on disk: wait for the debounced saves
  until(|| ids.iter().all(|id| std::fs::read_to_string(dir.path().join(format!("{id}.json"))).is_ok_and(|r| r.contains("question"))), 5000).await;

  let hub = RelayHub::start(Arc::new(Roster::new(dir.path())), Arc::new(|_: &str| {})).await.unwrap();
  let library: Weak<dyn SessionLibrary> = Arc::downgrade(&m.m) as _;
  hub.set_library(library);
  let session_b = m.m.live_session(&b).expect("B is open");
  let token = |thread: Option<&str>| hub.env(&session_b, thread, 1)[1]["value"].as_str().unwrap().to_owned();
  let (root, child) = (token(None), token(Some("thr")));
  let addr = hub.addr().to_owned();
  let link = |id: &str| format!("acpira://session/{id}");

  // The caller's project by default, every project with scope all; the caller itself is never listed to its own agent
  let hits = op(&addr, &root, search("zebracorn")).await.unwrap().unwrap();
  assert!(hits.contains(&link(&a)) && !hits.contains(&link(&c)), "{hits}");
  assert!(hits.contains("turn 1: zebracorn alpha question"), "{hits}");
  let all = HubOp::SearchSessions(SessionSearchArgs { query: Some("zebracorn".into()), scope: Some("all".into()), ..Default::default() });
  let hits = op(&addr, &root, all).await.unwrap().unwrap();
  assert!(hits.contains(&link(&a)) && hits.contains(&link(&c)), "{hits}");
  let recent = op(&addr, &root, search("")).await.unwrap().unwrap();
  assert!(recent.contains(&link(&a)) && !recent.contains(&link(&b)), "{recent}");

  // Reading: the outline, a page, a search inside the session; a link from another shell resolves too
  let outline = op(&addr, &root, read(&format!("vscode://hotic.acpira/session/{a}"))).await.unwrap().unwrap();
  assert!(outline.contains(&format!("Link: {}", link(&a))) && outline.contains("Turns: 1") && outline.contains("1. User: zebracorn alpha question"), "{outline}");
  let page = HubOp::ReadSession(SessionReadArgs { session: a.clone(), from: Some(1), to: Some(1), ..Default::default() });
  assert!(op(&addr, &root, page).await.unwrap().unwrap().contains("## Turn 1"));
  let inside = HubOp::SearchSessions(SessionSearchArgs { query: Some("ALPHA".into()), session: Some(a.clone()), ..Default::default() });
  assert!(op(&addr, &root, inside).await.unwrap().unwrap().contains("- turn 1 (user): zebracorn alpha question"));

  // A root agent asking for its own session is told it has it; the summoned child reads its parent
  assert_eq!(op(&addr, &root, read(&b)).await.unwrap().unwrap(), SELF_READ);
  let parent = op(&addr, &child, read(&b)).await.unwrap().unwrap();
  assert!(parent.contains("beta question"), "{parent}");
  assert!(op(&addr, &child, search("")).await.unwrap().unwrap().contains(&link(&b)));

  // An unknown session is an error the model can read; an unknown grant gets no answer at all
  assert!(op(&addr, &root, read("no-such-session")).await.unwrap().is_err());
  assert!(op(&addr, &root, read("two words")).await.unwrap().is_err());
  assert_eq!(op(&addr, "nope", read(&a)).await, None);
  m.dispose().await;
}
