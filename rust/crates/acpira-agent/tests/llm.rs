//! The model clients against the mock server: what goes on the wire and what comes back

use serde_json::json;

use acpira_agent::llm::{self, Endpoint, Event, Item, LlmError, Part, Request, StopReason};
use acpira_agent::mock::{self, MockModel, Reply};
use acpira_shared::providers::{Provider, ProviderModel, Sampling, Thinking};

fn endpoint(server: &MockModel) -> Endpoint {
  let provider: Provider = serde_json::from_value(json!({ "id": "mock", "baseUrl": server.base_url(), "headers": { "x-extra": "1" } })).unwrap();
  Endpoint::of(&provider, &ProviderModel::new("m1"), Some("sk-test")).unwrap()
}

fn request() -> Request {
  Request {
    model: "m1".into(),
    system: "be brief".into(),
    items: vec![Item::User(vec![Part::Text("hi".into())])],
    tools: vec![],
    max_tokens: Some(256),
    output_limit: None,
    sampling: Sampling::default(),
    thinking: Thinking::Auto,
    effort: None,
    serial_tools: false,
  }
}

async fn collect(server: &MockModel) -> Vec<Result<Event, LlmError>> {
  let mut rx = llm::stream(llm::default_http(), endpoint(server), request());
  let mut out = vec![];
  while let Some(e) = rx.recv().await {
    out.push(e);
  }
  out
}

#[tokio::test]
async fn streams_text_and_sends_the_key_and_headers() {
  let server = MockModel::start();
  server.push(mock::text("hello there"));
  let events = collect(&server).await;
  let text: String = events.iter().filter_map(|e| match e {
    Ok(Event::Text(t)) => Some(t.as_str()),
    _ => None,
  }).collect();
  assert_eq!(text, "hello there");
  let Some(Ok(Event::Done { stop, usage, .. })) = events.last() else { panic!("{events:?}") };
  assert_eq!(*stop, StopReason::EndTurn);
  assert_eq!(usage.unwrap().input, 100);
  let req = &server.requests()[0];
  assert_eq!(req.path, "/v1/chat/completions");
  assert_eq!(req.header("authorization"), Some("Bearer sk-test"));
  assert_eq!(req.header("x-extra"), Some("1"));
  assert_eq!(req.body["max_tokens"], 256);
  assert_eq!(req.body["messages"][0], json!({ "role": "system", "content": "be brief" }));
}

#[tokio::test]
async fn http_errors_carry_the_provider_message() {
  let server = MockModel::start();
  server.push(Reply::Status(401, r#"{"error":{"message":"Invalid API key"}}"#.into()));
  let events = collect(&server).await;
  assert!(matches!(&events[..], [Err(LlmError::Http { status: 401, message, .. })] if message == "Invalid API key"), "{events:?}");
}

#[tokio::test]
async fn a_cut_stream_is_a_protocol_error() {
  let server = MockModel::start();
  server.push(Reply::Cut(vec![mock::delta(json!({ "content": "par" }))]));
  let events = collect(&server).await;
  assert!(matches!(events.last(), Some(Err(LlmError::Protocol(_)))), "{events:?}");
}

#[tokio::test]
async fn the_anthropic_client_sends_its_headers_and_assembles_a_tool_call() {
  let server = MockModel::start();
  server.push(mock::messages(&[json!({ "type": "text", "text": "Reading." }), json!({ "type": "tool_use", "id": "toolu_1", "name": "read", "input": { "path": "a.rs" } })], "tool_use", 60));
  let provider: Provider = serde_json::from_value(json!({ "id": "mock", "format": "anthropic", "baseUrl": server.base_url() })).unwrap();
  let endpoint = Endpoint::of(&provider, &ProviderModel::new("claude-mock"), Some("sk-ant")).unwrap();
  let mut rx = llm::stream(llm::default_http(), endpoint, request());
  let mut events = vec![];
  while let Some(e) = rx.recv().await {
    events.push(e.unwrap());
  }
  let req = &server.requests()[0];
  assert_eq!(req.path, "/v1/messages");
  assert_eq!((req.header("x-api-key"), req.header("anthropic-version")), (Some("sk-ant"), Some("2023-06-01")));
  assert_eq!(req.header("authorization"), Some("Bearer sk-ant"), "not api.anthropic.com: the bearer form too");
  assert_eq!(req.body["max_tokens"], 256);
  assert_eq!(req.body["system"][0]["text"], "be brief");
  assert_eq!(req.body["messages"], json!([{ "role": "user", "content": [{ "type": "text", "text": "hi", "cache_control": { "type": "ephemeral" } }] }]));
  let Some(Event::Done { reply, stop, usage }) = events.last() else { panic!("{events:?}") };
  assert_eq!(*stop, StopReason::ToolUse);
  assert_eq!((reply.text.as_str(), reply.tool_calls[0].arguments.as_str()), ("Reading.", "{\"path\":\"a.rs\"}"));
  assert_eq!((usage.unwrap().input, usage.unwrap().cache_read), (100, 60));
}
