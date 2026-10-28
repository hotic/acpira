//! Exports: a session as Markdown or JSON under exports/, and the same Markdown copied as text

use anyhow::{Result, anyhow};

use acpira_shared::export_transcript::{ExportInput, ExportLabels, export_file_name, export_markdown, export_text};
use acpira_shared::protocol::ExportFormat;
use acpira_shared::transcript::*;

use super::{SessionManager, SessionSnapshot};
use crate::i18n::t;
use crate::store::transcript_store::RecordLoadError;
use crate::util::{local_stamp, now_iso};

impl SessionManager {
  /// Write the session as Markdown or JSON under exports/. Exporting a transcript that has not changed since the last export
  /// in the same format returns that file again (as long as it still exists) instead of writing a second copy
  /// A session's content as export, copy and read_session see it: the live session first, else its record, else a ChatGPT
  /// mirror. A record that cannot be read answers the same message the viewer would show
  pub async fn snapshot(&self, id: &str) -> Result<SessionSnapshot> {
    if let Some(c) = self.deps.chatgpt.as_ref().filter(|c| c.owns(id)) {
      return c.view(id).map(SessionSnapshot::Mirror).ok_or_else(|| anyhow!(self.record_load_message(id, &RecordLoadError::Missing)));
    }
    if let Some(l) = self.live(id) {
      return Ok(SessionSnapshot::Record(l.to_record()));
    }
    match self.deps.store.load_detailed(id).await {
      Ok(record) => Ok(SessionSnapshot::Record(record)),
      Err(error) => Err(anyhow!(self.record_load_message(id, &error))),
    }
  }

  pub(super) fn agent_name(&self, agent: &AgentId) -> String {
    self.agents().into_iter().find(|a| &a.id == agent).map(|a| a.name).unwrap_or_else(|| agent.clone())
  }

  /// The Markdown both "Export as Markdown" and "Copy as Markdown" produce, labels in the display language, blobs as paths
  fn snapshot_markdown(&self, id: &str, snap: &SessionSnapshot, agent_name: &str) -> String {
    let labels = ExportLabels {
      user: t("export.label.user"),
      agent: t("export.label.agent"),
      project: t("export.label.project"),
      exported: t("export.label.exported"),
      attachments: t("export.label.attachments"),
      thinking: t("export.label.thinking"),
      compacted: t("export.label.compacted"),
      auto_compact: t("export.label.autoCompact"),
      auto_continue: t("export.label.autoContinue"),
      auto_retry: t("export.label.autoRetry"),
      auto_gate: t("export.label.autoGate"),
      auto_subagents: t("export.label.autoSubagents"),
      error: t("export.label.error"),
    };
    let store = self.deps.store.clone();
    let sid = id.to_owned();
    export_markdown(
      &ExportInput { title: snap.title(), agent_name, cwd: snap.cwd(), exported_at: &now_iso(), turns: snap.turns() },
      &labels,
      &move |name| store.blob_path(&sid, name).map(|p| p.to_string_lossy().into_owned()),
    )
  }

  /// "Copy as Markdown": the export's Markdown as text, nothing written
  pub async fn session_markdown(&self, id: &str) -> Result<String> {
    let snap = self.snapshot(id).await?;
    Ok(self.snapshot_markdown(id, &snap, &self.agent_name(snap.agent())))
  }

  pub async fn export_session(&self, id: &str, format: ExportFormat) -> Result<std::path::PathBuf> {
    let mut exports = self.exports.lock().await;
    let snap = self.snapshot(id).await?;
    let json = snap.json()?;
    let agent_name = self.agent_name(snap.agent());
    // The fingerprint leaves out the export time stamped into the Markdown; the agent name and the label language are in it,
    // so a renamed agent or a switched display language still writes a fresh file
    let fingerprint = {
      use std::hash::{Hash, Hasher};
      let mut h = std::collections::hash_map::DefaultHasher::new();
      (&json, &agent_name, t("export.label.user")).hash(&mut h);
      h.finish()
    };
    let key = (id.to_owned(), format);
    if let Some((hash, path)) = exports.get(&key)
      && *hash == fingerprint
      && tokio::fs::try_exists(path).await.unwrap_or(false)
    {
      return Ok(path.clone());
    }
    let content = match format {
      ExportFormat::Json => json,
      ExportFormat::Markdown => self.snapshot_markdown(id, &snap, &agent_name),
      ExportFormat::Text => export_text(
        &ExportInput { title: snap.title(), agent_name: &agent_name, cwd: snap.cwd(), exported_at: "", turns: snap.turns() },
        &t("export.label.user"),
      ),
    };
    let path = self.deps.store.write_export(&export_file_name(snap.title(), format.extension(), &local_stamp()), &content).await?;
    exports.insert(key, (fingerprint, path.clone()));
    Ok(path)
  }
}
