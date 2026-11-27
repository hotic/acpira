//! Images: chunks, tool image items, resource links, generated images, data URLs, images in reply markdown

use super::*;

#[test]
fn an_image_chunk_closes_the_text_run_and_keeps_the_saved_blob_name() {
  let saved = Arc::new(Mutex::new(Vec::<String>::new()));
  let mut s = state();
  let log = saved.clone();
  s.ctx.save_image = saver(move |_data, mime| {
    let mut l = log.lock().unwrap();
    l.push(mime.to_owned());
    Some(format!("blob-{}.png", l.len()))
  });
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "here is " } }));
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "image", "data": PNG, "mimeType": "image/png" } }));
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "the dot" } }));
  expect_eq(&v(&s.turns[0])["blocks"], json!([
    { "type": "text", "markdown": "here is ", "streaming": false },
    { "type": "image", "id": "img-1", "mimeType": "image/png", "blob": "blob-1.png" },
    { "type": "text", "markdown": "the dot", "streaming": true },
  ]));
  assert_eq!(*saved.lock().unwrap(), ["image/png"]);
}

#[test]
fn without_a_saver_an_image_degrades_to_a_placeholder() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "image", "data": PNG, "mimeType": "image/png" } }));
  let mut s2 = state();
  s2.ctx.save_image = saver(|_, _| None);
  apply(&mut s2, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "image", "data": PNG, "mimeType": "image/tiff" } }));
  expect_match(block(&s, 0, 0), json!({ "type": "text", "markdown": "[image]" }));
  expect_match(block(&s2, 0, 0), json!({ "type": "text", "markdown": "[image: image/tiff, not shown]" }));
}

#[test]
fn a_tool_image_item_keeps_wire_order_splitting_the_text_run() {
  let mut s = state();
  s.ctx.save_image = saver(|_, _| Some("img.png".into()));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "im", "title": "view", "kind": "other", "status": "completed", "content": [
    { "type": "content", "content": { "type": "text", "text": "Revised prompt" } },
    { "type": "content", "content": { "type": "image", "data": PNG, "mimeType": "image/png", "uri": "/repo/red.png" } },
    { "type": "content", "content": { "type": "text", "text": "saved to disk" } },
  ] }));
  expect_match(block(&s, 0, 0), json!({ "contents": [
    { "type": "text", "text": "Revised prompt" },
    { "type": "image", "mimeType": "image/png", "blob": "img.png", "uri": "/repo/red.png" },
    { "type": "text", "text": "saved to disk" },
  ] }));
  // A lone image is the primary content — no contents list needed
  let mut s2 = state();
  s2.ctx.save_image = saver(|_, _| Some("img.png".into()));
  apply(&mut s2, json!({ "sessionUpdate": "tool_call", "toolCallId": "im", "title": "view", "kind": "other", "status": "completed",
    "content": [{ "type": "content", "content": { "type": "image", "data": PNG, "mimeType": "image/png" } }] }));
  expect_match(block(&s2, 0, 0), json!({ "content": { "type": "image", "blob": "img.png" } }));
  expect_absent(block(&s2, 0, 0), "contents");
}

#[test]
fn a_resource_link_to_a_local_image_becomes_an_image_and_unreadable_stays_a_link() {
  use base64::Engine;
  let dir = tempfile::tempdir().unwrap();
  let png_path = dir.path().join("shot.png");
  std::fs::write(&png_path, base64::engine::general_purpose::STANDARD.decode(PNG).unwrap()).unwrap();
  let png = png_path.to_string_lossy().into_owned();
  let seen = Arc::new(Mutex::new(Vec::<String>::new()));
  let mut s = state();
  let log = seen.clone();
  s.ctx.save_image_file = Some(Arc::new(move |p: &str| {
    log.lock().unwrap().push(p.to_owned());
    Some("file.png".into())
  }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "v", "title": "view_image", "kind": "read", "status": "completed",
    "content": [{ "type": "content", "content": { "type": "resource_link", "name": "shot.png", "uri": png } }] }));
  assert_eq!(*seen.lock().unwrap(), [png.clone()]);
  expect_match(block(&s, 0, 0), json!({ "content": { "type": "image", "mimeType": "image/png", "blob": "file.png", "uri": png } }));
  // A file:// uri resolves the same way
  let mut s2 = state();
  s2.ctx.save_image_file = Some(Arc::new(|_: &str| Some("f2.png".into())));
  apply(&mut s2, json!({ "sessionUpdate": "tool_call", "toolCallId": "v", "title": "view_image", "kind": "read", "status": "completed",
    "content": [{ "type": "content", "content": { "type": "resource_link", "name": "shot.png", "uri": format!("file://{png}") } }] }));
  expect_match(block(&s2, 0, 0), json!({ "content": { "type": "image", "blob": "f2.png" } }));
  // Non-image links, remote links and a saver that refuses all keep the link's text rendering
  let mut s3 = state();
  s3.ctx.save_image_file = Some(Arc::new(|_: &str| None));
  apply(&mut s3, json!({ "sessionUpdate": "tool_call", "toolCallId": "v", "title": "view", "kind": "read", "status": "completed", "content": [
    { "type": "content", "content": { "type": "resource_link", "name": "nope.png", "uri": "/missing/nope.png" } },
    { "type": "content", "content": { "type": "resource_link", "name": "notes", "uri": "/repo/notes.md" } },
    { "type": "content", "content": { "type": "resource_link", "name": "remote", "uri": "https://x.io/pic.png" } },
  ] }));
  expect_match(block(&s3, 0, 0), json!({ "content": { "type": "text", "text": "/missing/nope.png\n/repo/notes.md\nhttps://x.io/pic.png" } }));
}

#[test]
fn codex_image_generation_is_named_by_identity_and_keeps_the_revised_prompt_and_image() {
  let mut s = state();
  s.ctx.save_image = saver(|_, _| Some("gen.png".into()));
  // codex-acp 1.13.0 createImageGenerationStartUpdate / createImageGenerationCompleteUpdate
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "ig_1", "kind": "other", "title": "Image generation",
    "status": "in_progress", "rawInput": { "id": "ig_1" } }));
  expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.imagegen", "kind": "other", "status": "in_progress" }));
  // The title is the tool's name, never a target
  expect_absent(block(&s, 0, 0), "target");
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "ig_1", "status": "completed", "content": [
    { "type": "content", "content": { "type": "text", "text": "Revised prompt: a red dot" } },
    { "type": "content", "content": { "type": "image", "data": PNG, "mimeType": "image/png", "uri": "/Users/x/.codex/generated_images/t/ig_1.png" } },
  ], "rawOutput": { "status": "completed", "revisedPrompt": "a red dot", "result": PNG, "savedPath": "/Users/x/.codex/generated_images/t/ig_1.png" } }));
  expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.imagegen", "status": "completed", "contents": [
    { "type": "text", "text": "Revised prompt: a red dot" },
    { "type": "image", "blob": "gen.png", "uri": "/Users/x/.codex/generated_images/t/ig_1.png" },
  ] }));
}

#[test]
fn an_image_generation_tool_that_answers_with_a_path_reads_the_file() {
  let seen = Arc::new(Mutex::new(Vec::<String>::new()));
  let mut s = state();
  let log = seen.clone();
  s.ctx.save_image_file = Some(Arc::new(move |p: &str| {
    log.lock().unwrap().push(p.to_owned());
    Some("saved.jpg".into())
  }));
  // Grok's image_gen by `x.ai/tool` name; its result names the absolute path (and a session-relative one the model repeats)
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "g1", "title": "imagine: a cat", "status": "in_progress",
    "rawInput": { "prompt": "a cat" }, "_meta": { "x.ai/tool": { "name": "image_gen" } } }));
  expect_match(block(&s, 0, 0), json!({ "verbKey": "verb.imagegen", "target": "a cat" }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "g1", "status": "completed", "content": [
    { "type": "content", "content": { "type": "text", "text": "Image saved to disk: /tmp/sess/images/1.jpg (images/1.jpg)" } },
  ] }));
  assert_eq!(*seen.lock().unwrap(), ["/tmp/sess/images/1.jpg"]);
  expect_match(block(&s, 0, 0), json!({ "contents": [
    { "type": "text", "text": "Image saved to disk: /tmp/sess/images/1.jpg (images/1.jpg)" },
    { "type": "image", "mimeType": "image/jpeg", "blob": "saved.jpg", "uri": "/tmp/sess/images/1.jpg" },
  ] }));
  // A later update does not attach the same file twice
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "g1", "status": "completed" }));
  assert_eq!(seen.lock().unwrap().len(), 1);
  // A rawOutput path works the same; an ordinary tool's path is never read
  let mut s2 = state();
  s2.ctx.save_image_file = Some(Arc::new(|_: &str| Some("r.png".into())));
  apply(&mut s2, json!({ "sessionUpdate": "tool_call", "toolCallId": "g2", "title": "image_edit", "status": "completed",
    "rawOutput": { "path": "/tmp/sess/images/2.png" } }));
  apply(&mut s2, json!({ "sessionUpdate": "tool_call", "toolCallId": "e", "title": "edit", "kind": "edit", "status": "completed",
    "content": [{ "type": "content", "content": { "type": "text", "text": "wrote /tmp/a.png" } }] }));
  // The image lands before the rawOutput text fallback, so the JSON receipt is not shown next to it
  expect_match(block(&s2, 0, 0), json!({ "verbKey": "verb.imagegen", "content": { "type": "image", "blob": "r.png", "uri": "/tmp/sess/images/2.png" } }));
  expect_absent(block(&s2, 0, 0), "contents");
  expect_match(block(&s2, 0, 1), json!({ "content": { "type": "text", "text": "wrote /tmp/a.png" } }));
}

#[test]
fn a_data_url_image_saves_the_decoded_payload_and_a_uri_only_image_keeps_the_reference() {
  let saved = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
  let mut s = state();
  let log = saved.clone();
  s.ctx.save_image = saver(move |data, mime| {
    log.lock().unwrap().push((data.to_owned(), mime.to_owned()));
    Some("i.png".into())
  });
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "image", "data": "", "mimeType": "", "uri": format!("data:image/png;base64,{PNG}") } }));
  let mut s2 = state();
  s2.ctx.save_image = saver(|_, _| Some("x.png".into()));
  apply(&mut s2, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "image", "data": "", "mimeType": "image/png", "uri": "/tmp/red.png" } }));
  assert_eq!(*saved.lock().unwrap(), [(PNG.to_owned(), "image/png".to_owned())]);
  expect_match(block(&s, 0, 0), json!({ "type": "image", "blob": "i.png", "uri": null }));
  expect_match(block(&s2, 0, 0), json!({ "type": "image", "blob": null, "uri": "/tmp/red.png" }));
}

#[test]
fn local_images_in_reply_markdown_are_read_once_even_when_the_syntax_spans_chunks() {
  let seen = Arc::new(Mutex::new(Vec::<String>::new()));
  let mut s = state();
  let log = seen.clone();
  s.ctx.cwd = Some("/work".into());
  s.ctx.save_image_file = Some(Arc::new(move |p: &str| {
    log.lock().unwrap().push(p.to_owned());
    (!p.contains("missing")).then(|| format!("{}.png", p.len()))
  }));
  let chunk = |s: &mut NormalizeState, text: &str| {
    apply(s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": text } }));
  };
  chunk(&mut s, "Here it is: ![shot](/tmp/sh");
  assert!(seen.lock().unwrap().is_empty());
  chunk(&mut s, "ot.png) and ![web](https://x.test/a.png) ");
  chunk(&mut s, "![rel](<docs/my shot.jpg> \"title\") ![again](/tmp/shot.png) ![gone](file:///tmp/missing.webp) ![text](/tmp/notes.txt)");
  assert_eq!(*seen.lock().unwrap(), ["/tmp/shot.png", "/work/docs/my shot.jpg", "/tmp/missing.webp"]);
  expect_match(block(&s, 0, 0), json!({ "type": "text", "images": [
    { "uri": "/tmp/shot.png", "blob": "13.png", "mimeType": "image/png" },
    { "uri": "docs/my shot.jpg", "mimeType": "image/jpeg" },
    { "uri": "file:///tmp/missing.webp", "mimeType": "image/webp" },
  ] }));
  // A source that could not be read is recorded without a blob and not retried by later chunks
  expect_absent(block(&s, 0, 0), "images.2.blob");
  chunk(&mut s, " done.");
  assert_eq!(seen.lock().unwrap().len(), 3);
}

#[test]
fn a_reply_image_with_a_multibyte_relative_path_resolves_against_the_cwd() {
  let seen = Arc::new(Mutex::new(Vec::<String>::new()));
  let mut s = state();
  let log = seen.clone();
  s.ctx.cwd = Some("/work".into());
  s.ctx.save_image_file = Some(Arc::new(move |p: &str| {
    log.lock().unwrap().push(p.to_owned());
    Some("cn.png".into())
  }));
  // The fifth byte of `截图.png` falls inside a character: the `file:` check must not slice there
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "![截图](截图.png)" } }));
  assert_eq!(*seen.lock().unwrap(), ["/work/截图.png"]);
  expect_match(block(&s, 0, 0), json!({ "images": [{ "uri": "截图.png", "blob": "cn.png" }] }));
}
