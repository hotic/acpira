//! Shared vendor and host extension message decoder for `ClientHandlers`.
//! Keeps `acp/transport/process.rs` vendor-free while giving IdleHandlers, SessionHandlers, and RelayHandlers
//! a single consistent decode path.

use serde_json::Value;

use crate::acp::transport::rpc::RpcError;
use crate::acp::vendors::{claude_auth, claude_workflow, grok};

/// Synchronous custom notification decoded on the transport reader task.
pub enum CustomNotification {
  Update(Value),
  AuthRetry(Value),
  ApiRetry(Value),
}

/// Decode a non-standard ACP notification synchronously in reader order.
pub fn decode_notification(method: &str, params: Value) -> Option<CustomNotification> {
  if method == claude_workflow::SDK_MESSAGE {
    if let Some(update) = claude_workflow::workflow_update(&params) {
      return Some(CustomNotification::Update(update));
    }
    if claude_auth::repeated_auth_retry(&params) {
      return Some(CustomNotification::AuthRetry(params));
    }
    return Some(CustomNotification::ApiRetry(params));
  }
  None
}

/// Decoded custom server-to-client RPC request.
pub enum CustomRequest {
  RemoteTool(Value),
  PluginHooks,
  PluginHook(Value),
  GrokQuestion(Value),
  GrokExitPlan(Value),
}

/// Decode a non-standard ACP server-to-client request.
pub fn decode_request(method: &str, params: Value) -> Result<CustomRequest, RpcError> {
  match method {
    acpira_shared::remote::TOOL => Ok(CustomRequest::RemoteTool(params)),
    acpira_agent::plugin_hooks::HOOKS => Ok(CustomRequest::PluginHooks),
    acpira_agent::plugin_hooks::HOOK => Ok(CustomRequest::PluginHook(params)),
    grok::GROK_ASK_QUESTION => grok::parse_question(&params).map(CustomRequest::GrokQuestion),
    grok::GROK_EXIT_PLAN => grok::parse_exit_plan(&params).map(CustomRequest::GrokExitPlan),
    _ => Err(RpcError::method_not_found(method)),
  }
}
