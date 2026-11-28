//! The host's skills for a stand-in CLI (docs/dev/local-run.md). The CLI scans only this disk, so the skills the host
//! has for the folder (`places`, read there as the session opens: the same set the built-in agent gets) reach it as a
//! list in the remote note, as commands in the composer's menu, and as their instructions behind a prompt that calls
//! one up. Nothing is copied into the stand-in, which stays empty

use std::path::Path;
use std::sync::Arc;

use acpira_agent::prompt::{HostFiles, Places};
use acpira_agent::skills::{self, Skill};
use acpira_shared::slash_commands::command_name;
use acpira_shared::transcript::{CommandInput, SlashCommand};
use serde_json::{Value, json};

use super::stand::Stand;

/// How much of a description one line of the note's list keeps
const DESCRIPTION_MAX: usize = 300;
/// How large the note's skill section may grow; skills past it are only counted
const LIST_MAX: usize = 16 * 1024;

/// The host's skills for the session folder (`places`), the project's over the host user's. A host user's skill whose
/// name this machine's `~/.agents/skills` (under `local_home`) also has is left out: the CLI lists that one itself, and
/// two of one name would leave `/name` ambiguous
pub fn of_host(host_cwd: &str, places: Option<&Value>, local_home: Option<&Path>) -> Vec<Skill> {
  let Some(files) = places.and_then(|p| serde_json::from_value::<HostFiles>(p.clone()).ok()) else { return vec![] };
  let host_user = files.home.as_ref().map(|h| h.join(".agents").join("skills"));
  let all = skills::scan(&Places::on_host(Path::new(host_cwd), Arc::new(files)));
  let local: Vec<String> = local_home.map(|h| skills::scan_dirs(&[h.join(".agents").join("skills")])).unwrap_or_default().into_iter().map(|s| s.name).collect();
  all.into_iter().filter(|s| !(host_user.as_ref().is_some_and(|h| s.path.starts_with(h)) && local.contains(&s.name))).collect()
}

/// The remote note's section on the host's skills: what each is for and where its SKILL.md is there; None without any
pub fn note(stand: &Stand, skills: &[Skill]) -> Option<String> {
  if skills.is_empty() {
    return None;
  }
  let mut out = format!(
    "## Skills on {alias}\n\nThe skills {alias} has for this project, which your own skill list does not show. When a task matches \
     one, or a user message calls one up as /name, read its SKILL.md with {prefix}Read first and follow it (a message that calls \
     one up may already carry its instructions). The files and commands a skill names are on {alias}, relative to its folder there.\n",
    alias = stand.alias,
    prefix = stand.profile.prefix,
  );
  let mut left = 0;
  for s in skills {
    let description = s.description.split_whitespace().collect::<Vec<_>>().join(" ");
    let line = format!("\n- {}: {} ({})", s.name, acpira_agent::budget::cut(&description, DESCRIPTION_MAX), s.path.display());
    if out.len() + line.len() > LIST_MAX {
      left += 1;
      continue;
    }
    out.push_str(&line);
  }
  if left > 0 {
    out.push_str(&format!("\n- {left} more, not listed: look in the skills folders above"));
  }
  Some(out)
}

/// The menu with the host's skills after everything already in it (`merged`, else the agent's `own`); a name already
/// there keeps its entry. Returns `merged` itself when no skill is added
pub fn with_commands(merged: Option<Vec<SlashCommand>>, own: &[SlashCommand], skills: &[Skill]) -> Option<Vec<SlashCommand>> {
  let extra: Vec<SlashCommand> = {
    let menu = merged.as_deref().unwrap_or(own);
    skills
      .iter()
      .filter(|s| !menu.iter().any(|c| c.name == s.name))
      .map(|s| SlashCommand { name: s.name.clone(), description: s.description.clone(), input: s.hint.clone().map(|hint| CommandInput { hint }), ..Default::default() })
      .collect()
  };
  if extra.is_empty() {
    return merged;
  }
  let mut all = merged.unwrap_or_else(|| own.to_vec());
  all.extend(extra);
  Some(all)
}

/// What a prompt that calls up host skills goes out as
#[derive(Debug, PartialEq)]
pub struct Expanded {
  /// The text the agent reads in place of the user's, when the prompt led with a skill's `/name`
  pub text: Option<String>,
  /// The skills' instructions, behind the prompt
  pub block: Value,
}

/// The host skills `text` calls up as `/name` (`skills::mentioned`, as the built-in agent reads them), leaving out names
/// the agent or a plugin answers itself (`taken`). A leading `/name` would reach the CLI as a command it does not know,
/// so the agent reads a request instead. None when no skill is called up, or the prompt is another command, which must
/// stay its only text
pub fn expand(text: &str, skills: &[Skill], taken: impl Fn(&str) -> bool, alias: &str) -> Option<Expanded> {
  let free: Vec<Skill> = skills.iter().filter(|s| !taken(&s.name)).cloned().collect();
  let mentions = skills::mentioned(text, &free);
  if mentions.is_empty() {
    return None;
  }
  let rewritten = match command_name(text) {
    Some(name) if mentions.iter().any(|(s, _)| s.name == name) => {
      let rest = text.get(name.len() + 1..).unwrap_or_default().trim();
      let lead = format!("Use the {name} skill below (its folder is on {alias}).");
      Some(if rest.is_empty() { lead } else { format!("{lead}\n\n{rest}") })
    }
    Some(_) => return None,
    None => None,
  };
  Some(Expanded { text: rewritten, block: json!({ "type": "text", "text": skills::expansion(&mentions).trim_start() }) })
}

/// Whether `text` leads with a host skill's `/name` (not `taken`): only the engine's rewrite makes it a prompt the CLI
/// reads, so it waits for the turn rather than being steered in as typed
pub fn leads_with(text: &str, skills: &[Skill], taken: impl Fn(&str) -> bool) -> bool {
  command_name(text).is_some_and(|n| !taken(n) && skills.iter().any(|s| s.name == n))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::acp::local_run::builtin;
  use std::path::PathBuf;

  fn skill(name: &str, path: &str, body: &str) -> Skill {
    Skill { name: name.into(), description: format!("{name} things"), hint: None, path: PathBuf::from(path), body: body.into() }
  }

  #[test]
  fn the_hosts_skills_come_from_places_without_a_local_twin() {
    let dir = tempfile::tempdir().unwrap();
    let (host_home, work, local_home) = (dir.path().join("host"), dir.path().join("host/proj"), dir.path().join("here"));
    std::fs::create_dir_all(work.join(".git")).unwrap();
    for (root, folder) in [(&host_home, "dig"), (&host_home, "notes"), (&work, "release"), (&work, "dig"), (&local_home, "notes")] {
      let d = root.join(".agents/skills").join(folder);
      std::fs::create_dir_all(&d).unwrap();
      std::fs::write(d.join("SKILL.md"), format!("---\ndescription: {folder} at {}\n---\nbody\n", root.display())).unwrap();
    }
    // As the helper sends them; nothing is read from the host's folders afterwards
    let places = serde_json::to_value(HostFiles::gather(&work, Some(&host_home))).unwrap();
    std::fs::remove_dir_all(&host_home).unwrap();
    let got = of_host(&work.to_string_lossy(), Some(&places), Some(&local_home));
    // The project's dig wins over the host user's; the host user's notes is already this machine's
    let seen: Vec<(&str, bool)> = got.iter().map(|s| (s.name.as_str(), s.path.starts_with(&work))).collect();
    assert_eq!(seen, [("dig", true), ("release", true)]);
    assert!(of_host("/x", None, None).is_empty());
  }

  #[test]
  fn the_note_lists_each_skill_with_its_host_path() {
    let stand = Stand::new(builtin("antigravity").and_then(|l| l.stand().cloned()).unwrap(), "example-host", "/home/u/proj", Path::new("/Users/dev/.acpira"));
    assert!(note(&stand, &[]).is_none());
    let text = note(&stand, &[skill("release", "/home/u/proj/.agents/skills/release/SKILL.md", "")]).unwrap();
    assert!(text.contains("## Skills on example-host") && text.contains("mcp_acpira_Read"), "{text}");
    assert!(text.contains("\n- release: release things (/home/u/proj/.agents/skills/release/SKILL.md)"), "{text}");
    let many: Vec<Skill> = (0..400).map(|i| skill(&format!("s{i}"), &format!("/home/u/proj/.agents/skills/s{i}/SKILL.md"), "")).collect();
    let text = note(&stand, &many).unwrap();
    assert!(text.len() <= LIST_MAX + 100 && text.contains("more, not listed"), "{}", text.len());
  }

  #[test]
  fn the_menu_adds_only_names_it_lacks() {
    let own = vec![SlashCommand { name: "dig".into(), description: "the CLI's own".into(), ..Default::default() }];
    let skills = [skill("dig", "/h/dig/SKILL.md", ""), Skill { hint: Some("version".into()), ..skill("release", "/h/release/SKILL.md", "") }];
    let all = with_commands(None, &own, &skills).unwrap();
    assert_eq!(all.iter().map(|c| (c.name.as_str(), c.description.as_str())).collect::<Vec<_>>(), [("dig", "the CLI's own"), ("release", "release things")]);
    assert_eq!(all[1].input.as_ref().map(|i| i.hint.as_str()), Some("version"));
    assert_eq!(with_commands(None, &own, &skills[..1]), None);
    let merged = Some(vec![SlashCommand { name: "release".into(), plugin: Some("p".into()), ..Default::default() }]);
    assert_eq!(with_commands(merged.clone(), &own, &skills[1..]), merged);
  }

  #[test]
  fn a_prompt_calling_up_a_host_skill_carries_its_instructions() {
    let skills = [skill("release", "/h/release/SKILL.md", "Tag $ARGUMENTS."), skill("dig", "/h/dig/SKILL.md", "Dig.")];
    let none = |_: &str| false;
    let x = expand("/release 1.2\nthen push", &skills, none, "example-host").unwrap();
    assert_eq!(x.text.as_deref(), Some("Use the release skill below (its folder is on example-host).\n\n1.2\nthen push"));
    let block = x.block["text"].as_str().unwrap();
    assert!(block.starts_with("<skill name=\"release\" path=\"/h/release/SKILL.md\">\nTag 1.2.\n</skill>"), "{block}");
    // Mid-sentence: the text stays as typed
    let x = expand("然后/dig 一下", &skills, none, "example-host").unwrap();
    assert!(x.text.is_none() && x.block["text"].as_str().unwrap().contains("Dig."));
    // The agent's own name, another command, or none at all
    assert!(expand("/dig", &skills, |n| n == "dig", "example-host").is_none());
    assert!(expand("/compact then /dig", &skills, none, "example-host").is_none());
    assert!(expand("plain text /tmp/release", &skills, none, "example-host").is_none());
    assert!(leads_with("/release 1", &skills, none) && !leads_with("/release 1", &skills, |n| n == "release") && !leads_with("go /release", &skills, none));
  }
}
