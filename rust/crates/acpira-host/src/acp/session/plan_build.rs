//! Executing an approved plan: apply the execution model, then release the approval card or leave plan mode and send the
//! implementation prompt

use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::Value;

use acpira_shared::plan_execution::plan_execution_prompt;
use acpira_shared::transcript::*;

use crate::acp::session::AcpSession;
use crate::acp::transcript::plans::{plan_documents, plan_documents_mut};
use crate::i18n::t;

impl AcpSession {
  /// Apply the selected execution model before releasing approval or dispatching an implementation turn
  pub async fn build_plan(self: &Arc<Self>, plan_id: &str, model: Option<(String, String)>, option_id: Option<String>) -> Result<()> {
    let (plan, permission, option) = {
      let c = self.core.lock();
      if c.building_plan || c.status != SessionStatus::Ready {
        return Ok(());
      }
      let Some(plan) = plan_documents(&c.state.turns).into_iter().find(|p| p.id == plan_id).cloned() else { return Ok(()) };
      if plan.markdown.is_empty() || plan.status == PlanDocStatus::Executing {
        return Ok(());
      }
      let permission = Self::permission_by_plan(&c, plan_id);
      if c.phase.running && permission.is_none() {
        return Ok(());
      }
      // An expired approval click must never become a fresh implementation prompt
      if option_id.is_some() && permission.is_none() {
        return Ok(());
      }
      let kind = |o: &Value| o.get("kind").and_then(Value::as_str).unwrap_or("").to_owned();
      let option = permission.as_ref().and_then(|(_, opts)| match &option_id {
        Some(id) => opts
          .iter()
          .find(|o| {
            o.get("optionId").and_then(Value::as_str) == Some(id.as_str())
              && (kind(o).starts_with("allow") || kind(o).starts_with("reject"))
          })
          .cloned(),
        None => opts.iter().find(|o| kind(o) == "allow_once").cloned(),
      });
      if permission.is_some() && option.is_none() {
        return Err(anyhow!(t("host.planOptionsStale")));
      }
      (plan, permission, option)
    };
    self.core.lock().building_plan = true;
    let result: Result<()> = async {
      let option_id = option.as_ref().and_then(|o| o.get("optionId").and_then(Value::as_str)).map(str::to_owned);
      let reject = option.as_ref().and_then(|o| o.get("kind").and_then(Value::as_str)).is_some_and(|k| k.starts_with("reject"));
      if let (Some((block, _)), true) = (&permission, reject) {
        let mut c = self.core.lock();
        if c.perms.pending.iter().any(|p| &p.block_id == block) {
          self.resolve_permission_locked(&mut c, block, option_id.as_deref().unwrap_or(""));
        }
        return Ok(());
      }
      if let Some((config_id, value)) = &model {
        let current = {
          let c = self.core.lock();

          c.state.controls.options.iter().find(|x| &x.id == config_id && x.category.as_deref() == Some("model")).cloned()
        };
        let Some(ctl) = current.filter(|x| x.options.iter().any(|o| &o.id == value)) else {
          return Err(anyhow!(t("host.executorUnavailable")));
        };
        if ctl.value.as_ref() != Some(value) {
          self.set_config(config_id.clone(), value.clone()).await?;
        }
      }
      if self.status() != SessionStatus::Ready {
        return Ok(());
      }
      if let Some((block, _)) = &permission {
        let mut c = self.core.lock();
        if c.perms.pending.iter().any(|p| &p.block_id == block) {
          self.resolve_permission_locked(&mut c, block, option_id.as_deref().unwrap_or(""));
        }
        return Ok(());
      }
      let (running, mode, in_plan) = {
        let c = self.core.lock();
        let mode = c
          .state
          .controls
          .modes
          .iter()
          .find(|m| ["default", "accept-edits", "agent", "code"].contains(&m.id.as_str()))
          .map(|m| m.id.clone());
        (c.phase.running, mode, c.state.controls.mode_id.as_deref() == Some("plan"))
      };
      if running {
        return Ok(());
      }
      if in_plan {
        let mode = mode.ok_or_else(|| anyhow!(t("host.noExecutableMode")))?;
        self.set_mode(mode).await?;
      }
      {
        let mut c = self.core.lock();
        if c.status != SessionStatus::Ready || c.phase.running {
          return Ok(());
        }
        if let Some(p) = plan_documents_mut(&mut c.state.turns).into_iter().find(|p| p.id == plan_id) {
          p.status = PlanDocStatus::Executing;
        }
      }
      // Model-facing instruction: fixed English regardless of UI language
      self.prompt(plan_execution_prompt(&plan.markdown), vec![], false, None, Some(plan.id.clone())).await;
      Ok(())
    }
    .await;
    let mut c = self.core.lock();
    c.building_plan = false;
    self.touch(&mut c);
    result
  }
}
