//! Steering: a message the client sends into the running turn (`_session/steering`, `acp::Server::steer`). It waits in
//! `SessionState::steers` and reaches the model with the turn's next call, as one user message behind the round's tool
//! results; a reply that would have ended the turn lets the turn go on instead, so the model answers it in the same
//! turn. A running tool or a streaming reply is never cut short for it, which is what Claude Code and Codex do with a
//! message typed while they work. A message no call carried (the turn was cancelled, failed or stopped at a limit) goes
//! into the history when the turn ends, so the next turn's model still gets what the user sent.

use serde_json::{Value, json};

use crate::acp::{Server, Session};
use crate::llm::{Item, Part, STEER_OPEN as OPEN};
use crate::skills::{self, Skill};

const CLOSE: &str = "\n</steer>";

/// What the model is told in front of the user's words
const NOTE: &str = "The user wrote this while you were working. Take it into account for the task in progress and carry on from \
where you are, unless it asks you to stop or to do something else.";

/// The messages as one user message: the note, then each message's parts the way a prompt's are made (attachments, and
/// images when the model takes them), with the skills it mentions expanded behind it like a prompt's
pub fn item(prompts: &[Value], images: bool, skills: &[Skill]) -> Item {
  let mut parts = vec![Part::Text(format!("{OPEN}{NOTE}{CLOSE}"))];
  for prompt in prompts {
    parts.extend(crate::turn::prompt_parts(prompt, images).into_iter().filter(|p| !matches!(p, Part::Text(t) if t.is_empty())));
    let expansion = skills::expansion(&skills::mentioned(&crate::turn::prompt_text(prompt), skills));
    if !expansion.is_empty() {
      parts.push(Part::Text(expansion.trim_start().to_owned()));
    }
  }
  Item::User(crate::turn::merge_text(parts))
}

/// The user's own words in a steered message (`Item::is_steer`): its text without the note (images left out)
pub fn text_of(item: &Item) -> Option<String> {
  let Item::User(parts) = item else { return None };
  if !item.is_steer() {
    return None;
  }
  let text = parts
    .iter()
    .filter_map(|p| match p {
      Part::Text(t) => Some(t.as_str()),
      Part::Image { .. } => None,
    })
    .collect::<Vec<_>>()
    .join("\n\n");
  let words = text.split_once(CLOSE).map_or(text.as_str(), |(_, rest)| rest);
  Some(words.trim().to_owned())
}

/// Take the messages steered in since the last call into the history, in front of the next call
pub fn take(session: &Session, images: bool, skills: &[Skill]) {
  let mut st = session.state.lock();
  if st.steers.is_empty() {
    return;
  }
  let prompts = std::mem::take(&mut st.steers);
  let item = item(&prompts, images, skills);
  session.store.item(&item);
  record(session, &prompts);
  st.items.push(item);
}

/// Whether the turn may end: no steered message waits for the model. When it may, it takes no more of them, decided
/// under the same lock as the check, so a steer from now on is handed back instead of being left behind by a turn
/// that is over
pub fn may_end(session: &Session) -> bool {
  let mut st = session.state.lock();
  if !st.steers.is_empty() {
    return false;
  }
  st.steerable = false;
  true
}

/// Close steering at a turn's end. The messages no call carried come back as the user message to keep in the history
/// for the next turn (with images when the session's model takes them now, since that model is the one to get it),
/// with their content blocks for the replay (`record`)
pub fn close(server: &Server, session: &Session) -> Option<(Item, Vec<Value>)> {
  let (prompts, pick) = {
    let mut st = session.state.lock();
    st.steerable = false;
    (std::mem::take(&mut st.steers), st.model.clone())
  };
  if prompts.is_empty() {
    return None;
  }
  let images = pick.and_then(|p| crate::turn::limits(&server.config.get(), &server.catalog.get(), &p)).is_some_and(|m| m.takes_images());
  let places = crate::prompt::Places::new(&session.cwd, crate::turn::user_home().as_deref());
  Some((item(&prompts, images, &skills::scan(&places)), prompts))
}

/// The replay shows steered messages where the model got them, as the user sent them (without the note); the
/// engine's load replay makes that a user turn
pub fn record(session: &Session, prompts: &[Value]) {
  for (i, prompt) in prompts.iter().enumerate() {
    if i > 0 {
      session.store.record_update(&json!({ "sessionUpdate": "user_message_chunk", "content": { "type": "text", "text": "\n\n" } }));
    }
    for block in prompt.as_array().into_iter().flatten() {
      session.store.record_update(&json!({ "sessionUpdate": "user_message_chunk", "content": block }));
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn text(t: &str) -> Value {
    json!([{ "type": "text", "text": t }])
  }

  #[test]
  fn steered_messages_become_one_marked_user_message() {
    let item = item(&[text("use printf"), json!([{ "type": "text", "text": "and" }, { "type": "image", "data": "AA", "mimeType": "image/png" }])], true, &[]);
    let Item::User(parts) = &item else { panic!("{item:?}") };
    let [Part::Text(t), Part::Image { .. }] = &parts[..] else { panic!("{parts:?}") };
    assert!(t.starts_with("<steer>\nThe user wrote this while you were working."), "{t}");
    assert!(t.ends_with("</steer>\n\nuse printf\n\nand"), "{t}");
    assert!(item.is_steer());
    assert_eq!(text_of(&item).as_deref(), Some("use printf\n\nand"));
    // An ordinary prompt is not one, whatever it says
    let prompt = Item::User(vec![Part::Text("please <steer> this".into())]);
    assert!(!prompt.is_steer());
    assert_eq!(text_of(&prompt), None);
  }

  #[test]
  fn a_model_without_images_gets_the_note_for_an_image() {
    let item = item(&[json!([{ "type": "image", "data": "AA", "mimeType": "image/png" }])], false, &[]);
    let Item::User(parts) = &item else { panic!() };
    let [Part::Text(t)] = &parts[..] else { panic!("{parts:?}") };
    assert!(t.ends_with("</steer>\n\n[An image was attached, but this model does not accept images.]"), "{t}");
  }

  #[test]
  fn a_mentioned_skill_is_expanded_behind_the_message() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("SKILL.md");
    std::fs::write(&path, "---\nname: greet\n---\nSay hello to $ARGUMENTS.").unwrap();
    let skill = Skill { name: "greet".into(), description: String::new(), hint: None, path };
    let item = item(&[text("then /greet Bob")], true, &[skill]);
    let Item::User(parts) = &item else { panic!() };
    let [Part::Text(t)] = &parts[..] else { panic!("{parts:?}") };
    assert!(t.contains("then /greet Bob\n\n<skill name=\"greet\""), "{t}");
    assert!(t.contains("Say hello to Bob."), "{t}");
  }
}
