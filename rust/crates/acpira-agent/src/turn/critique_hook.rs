//! Per-turn Critique coordinator inside `acpira-agent`:
//! - Collects chronological write & verifier actions plus User Interaction Carryover (`question` answers).
//! - Holds `exit_plan` before opening the user approval card (`StagedTerminal`) to run at most one zero-tool
//!   cross-family Plan Critique and allow the Lead to Revise Once within the same prompt cache.
//! - Optionally runs one post-edit Draft Critique before committing a turn that modified the workspace,
//!   honoring the 4 zero-cost short-circuit skip rules and writing structured `critique` / `critique_commit` events.

use std::collections::HashSet;
use std::sync::Arc;

use acpira_rpc::cancel::Cancel;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::setup::Setup;
use crate::acp::{Server, Session};
use crate::critique::evidence::{
  DefaultSecretFilter, EvidencePacket, InteractionKind, PrimaryAction, RenderBudget, UserInteraction, is_verifier_command, populate_git_evidence,
};
use crate::critique::peer::resolve_peer;
use crate::critique::wire::{Checkpoint, CritiqueOutcome, SkipInputs, revision_handoff, short_circuit_skip};
use crate::critique::{log_event, run_zero_tool};
use crate::llm::Endpoint;
use crate::modes;
use crate::tools::{self, Action, Output};

const MAX_RECORDED_ACTIONS: usize = 16;
const MAX_ACTION_OUTPUT_BYTES: usize = 1500;

/// Per-turn state for implicit zero-tool Critique (isolated from `SessionState` lock contention)
pub(super) struct TurnCritique {
  original_task: String,
  start_model: String,
  has_workspace_writes: bool,
  actions: Vec<PrimaryAction>,
  interactions: Vec<UserInteraction>,
  /// Digests of `plan.md` already critiqued in this turn (enforces max 1 Revise Once per plan pass)
  plan_critiqued_digests: HashSet<String>,
  pub(super) plan_reviewed: bool,
  draft_critiqued: bool,
}

impl TurnCritique {
  pub(super) fn new(original_task: String, start_model: String) -> Self {
    Self {
      original_task,
      start_model,
      has_workspace_writes: false,
      actions: vec![],
      interactions: vec![],
      plan_critiqued_digests: HashSet::new(),
      plan_reviewed: false,
      draft_critiqued: false,
    }
  }

  /// Record write and verifier tool executions for the bounded evidence packet and `read_only_first_pass` check
  pub(super) fn record_tool_outcome(&mut self, action: &Action, out: &Output) {
    match action {
      Action::Write { files } => {
        if !out.is_error && files.iter().any(|f| !f.unchanged()) {
          self.has_workspace_writes = true;
        }
        let summary = files.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>().join(", ");
        self.push_action(PrimaryAction {
          tool: "write/edit".into(),
          summary,
          output: crate::budget::cut(&out.model, MAX_ACTION_OUTPUT_BYTES).to_owned(),
          is_error: out.is_error,
          is_verifier: false,
        });
      }
      Action::Bash { command, background, .. } => {
        let verifier = is_verifier_command(command);
        // Non-background commands other than pure read-only verifiers/inspections may mutate the workspace
        if !background && !looks_pure_readonly_shell(command) {
          self.has_workspace_writes = true;
        }
        if verifier || out.is_error {
          self.push_action(PrimaryAction {
            tool: "bash".into(),
            summary: crate::budget::cut(command, 240).to_owned(),
            output: crate::budget::cut(&out.model, MAX_ACTION_OUTPUT_BYTES).to_owned(),
            is_error: out.is_error,
            is_verifier: verifier,
          });
        }
      }
      _ => {}
    }
  }

  fn push_action(&mut self, action: PrimaryAction) {
    if self.actions.len() >= MAX_RECORDED_ACTIONS {
      self.actions.remove(0);
    }
    self.actions.push(action);
  }

  /// Record settled user question answers as User Interaction Carryover
  pub(super) fn record_question(&mut self, questions: &[tools::question::Question], response: &str) {
    let prompt = questions.iter().map(|q| q.question.as_str()).collect::<Vec<_>>().join(" | ");
    self.interactions.push(UserInteraction {
      kind: InteractionKind::Question,
      prompt: crate::budget::cut(&prompt, 400).to_owned(),
      response: crate::budget::cut(response, 800).to_owned(),
    });
  }

  /// Record a plan approval decision into carryover for downstream draft checks
  pub(super) fn record_plan_decision(&mut self, approved: bool) {
    self.interactions.push(UserInteraction {
      kind: InteractionKind::PlanApproval,
      prompt: "Plan approval card".into(),
      response: if approved { "User approved the plan (Build)".into() } else { "User requested plan revision".into() },
    });
  }

  /// Pre-approval-card Plan Critique (holds `exit_plan` card when a grounded defect is found, at most once per turn).
  /// Returns `Some(revision_handoff)` when Lead must Revise Once before showing the card, or `None` to open the card.
  pub(super) async fn review_plan(
    &mut self,
    server: &Arc<Server>,
    session: &Arc<Session>,
    setup: &Setup,
    plan_text: &str,
    cancel: &Cancel,
  ) -> Option<String> {
    if setup.raw || !self.plan_critiqued_digests.is_empty() {
      return None;
    }
    let digest = short_digest(plan_text);
    if self.plan_critiqued_digests.contains(&digest) {
      return None;
    }
    let selection_changed = session.state.lock().model.as_deref().is_some_and(|m| m != self.start_model);
    let skip_inputs = SkipInputs {
      user_halted: cancel.is_cancelled(),
      selection_changed,
      awaiting_background_work: session.jobs.has_running(),
      has_workspace_writes: true,
    };
    if let Some(reason) = short_circuit_skip(Checkpoint::Plan, skip_inputs) {
      log_event(&session.store, Checkpoint::Plan, &CritiqueOutcome::Skipped(reason));
      return None;
    }

    let config = session.config();
    let catalog = server.catalog.get();
    let sidekick_pick = session.state.lock().sidekick.clone();
    let Some((peer_provider, peer_model, peer_label)) = resolve_peer(&config, &catalog, &setup.pick, sidekick_pick.as_deref(), None) else {
      log_event(&session.store, Checkpoint::Plan, &CritiqueOutcome::NoPeer);
      return None;
    };
    let Some(endpoint) = Endpoint::of(peer_provider, peer_model, config.api_key(&peer_provider.id)) else {
      log_event(&session.store, Checkpoint::Plan, &CritiqueOutcome::NoPeer);
      return None;
    };

    let mut packet = EvidencePacket::empty(Checkpoint::Plan);
    packet.plan_text = Some(plan_text.to_owned());
    packet.user_interactions = self.interactions.clone();
    packet.primary_actions = self.actions.clone();
    packet.verifier_evidence = self.actions.iter().filter(|a| a.is_verifier).cloned().collect();

    let outcome = run_zero_tool(
      server.http.clone(),
      endpoint,
      peer_model.id.clone(),
      peer_label,
      Checkpoint::Plan,
      &self.original_task,
      plan_text,
      &packet,
      Some(cancel.clone()),
    )
    .await;
    log_event(&session.store, Checkpoint::Plan, &outcome);
    self.plan_critiqued_digests.insert(digest);

    match outcome {
      CritiqueOutcome::Revise { feedback, defect, .. } => {
        self.plan_reviewed = true;
        Some(revision_handoff(Checkpoint::Plan, &feedback, &defect, &self.interactions))
      }
      CritiqueOutcome::Approved { .. } => {
        self.plan_reviewed = true;
        None
      }
      CritiqueOutcome::Invalid { .. } | CritiqueOutcome::NoPeer | CritiqueOutcome::Skipped(_) => None,
    }
  }

  /// Post-edit Draft Critique before committing the final reply of a turn (runs at most once per turn).
  /// Returns `Some(revision_handoff)` when Lead must Revise Once, or `None` to commit the draft idempotently.
  pub(super) async fn review_draft(
    &mut self,
    server: &Arc<Server>,
    session: &Arc<Session>,
    setup: &Setup,
    draft_text: &str,
    user_halted: bool,
    cancel: &Cancel,
  ) -> Option<String> {
    if setup.raw || self.draft_critiqued || session.state.lock().mode == modes::PLAN {
      return None;
    }
    self.draft_critiqued = true;

    let selection_changed = session.state.lock().model.as_deref().is_some_and(|m| m != self.start_model);
    let skip_inputs = SkipInputs {
      user_halted: user_halted || cancel.is_cancelled(),
      selection_changed,
      awaiting_background_work: session.jobs.has_running(),
      has_workspace_writes: self.has_workspace_writes,
    };
    if let Some(reason) = short_circuit_skip(Checkpoint::Draft, skip_inputs) {
      log_event(&session.store, Checkpoint::Draft, &CritiqueOutcome::Skipped(reason));
      return None;
    }

    let config = session.config();
    let catalog = server.catalog.get();
    let sidekick_pick = session.state.lock().sidekick.clone();
    let Some((peer_provider, peer_model, peer_label)) = resolve_peer(&config, &catalog, &setup.pick, sidekick_pick.as_deref(), None) else {
      log_event(&session.store, Checkpoint::Draft, &CritiqueOutcome::NoPeer);
      return None;
    };
    let Some(endpoint) = Endpoint::of(peer_provider, peer_model, config.api_key(&peer_provider.id)) else {
      log_event(&session.store, Checkpoint::Draft, &CritiqueOutcome::NoPeer);
      return None;
    };

    let mut packet = EvidencePacket::empty(Checkpoint::Draft);
    populate_git_evidence(&mut packet, &session.cwd, RenderBudget::default(), &DefaultSecretFilter);
    packet.user_interactions = self.interactions.clone();
    packet.primary_actions = self.actions.clone();
    packet.verifier_evidence = self.actions.iter().filter(|a| a.is_verifier).cloned().collect();

    let outcome = run_zero_tool(
      server.http.clone(),
      endpoint,
      peer_model.id.clone(),
      peer_label,
      Checkpoint::Draft,
      &self.original_task,
      draft_text,
      &packet,
      Some(cancel.clone()),
    )
    .await;
    log_event(&session.store, Checkpoint::Draft, &outcome);

    match outcome {
      CritiqueOutcome::Revise { feedback, defect, .. } => {
        session.store.append(json!({ "type": "critique_commit", "checkpoint": "draft", "revised": true }));
        Some(revision_handoff(Checkpoint::Draft, &feedback, &defect, &self.interactions))
      }
      CritiqueOutcome::Approved { .. } | CritiqueOutcome::Invalid { .. } => {
        session.store.append(json!({ "type": "critique_commit", "checkpoint": "draft", "revised": false }));
        None
      }
      CritiqueOutcome::NoPeer | CritiqueOutcome::Skipped(_) => None,
    }
  }
}

fn short_digest(text: &str) -> String {
  let mut h = Sha256::new();
  h.update(text.as_bytes());
  h.finalize().iter().take(6).map(|b| format!("{b:02x}")).collect()
}

fn looks_pure_readonly_shell(cmd: &str) -> bool {
  let t = cmd.trim();
  if t.contains('>') || t.contains(" -i") || t.contains("rm ") || t.contains("mv ") || t.contains("cp ") || t.contains("mkdir ") || t.contains("touch ") {
    return false;
  }
  let ro = ["ls", "cat ", "head ", "tail ", "rg ", "grep ", "find ", "git status", "git diff", "git log", "git show", "pwd", "wc ", "which "];
  ro.iter().any(|p| t == *p || t.starts_with(p))
}
