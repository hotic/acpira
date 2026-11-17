use super::*;
use acpira_shared::shared_config::PrivateMatch;
use links::Spot;

// A link point wired to `target` in whatever form the platform made: a symlink, or on Windows without symlink
// rights a junction (directories) or a hard link (files). `is_symlink` alone would only hold on some machines
fn wired(at: &Path, target: &Path) -> bool {
  links::inspect(at, target) == Spot::Linked
}

fn setup() -> (tempfile::TempDir, Places, Arc<SharedConfig>, Vec<String>) {
  let t = tempfile::tempdir().unwrap();
  let home = t.path().join("home");
  let root = t.path().join("repo");
  std::fs::create_dir_all(root.join(".git/info")).unwrap();
  for d in [".claude", ".codex", ".agents/skills/dig", ".grok"] {
    std::fs::create_dir_all(home.join(d)).unwrap();
  }
  std::fs::write(home.join(".agents/skills/dig/SKILL.md"), "---\nname: dig\ndescription: d\n---\n").unwrap();
  std::fs::create_dir_all(root.join(".agents/skills/release")).unwrap();
  std::fs::write(root.join(".agents/skills/release/SKILL.md"), "---\nname: release\n---\n").unwrap();
  let places = Places { home: home.clone(), config: home.join(".config"), root: Some(root) };
  let log: LogFn = Arc::new(|l| eprintln!("{l}"));
  let cfg = SharedConfig::new(t.path().join("data"), log);
  let agents = ["claude", "codex", "grok"].map(String::from).to_vec();
  (t, places, cfg, agents)
}

fn caps() -> crate::util::Lookup<McpCaps> {
  Arc::new(|_| None)
}

fn pick(at: &Path, choice: Choice) -> Pick {
  Pick { at: s(at), choice }
}

#[tokio::test]
async fn project_links_come_on_their_own_and_user_links_follow_the_panel() {
  let (_t, p, cfg, agents) = setup();
  let home = p.home.clone();
  let root = p.root.clone().unwrap();
  std::fs::write(home.join(".agents/AGENTS.md"), "# shared\n").unwrap();
  std::fs::write(home.join(".codex/AGENTS.md"), "# shared\n").unwrap();
  std::fs::write(home.join(".grok/AGENTS.md"), "# mine\n").unwrap();

  // Looking is enough for the project: Claude's link to .agents/skills appears, out of git status
  let v = cfg.view(p.clone(), agents.clone(), caps()).await;
  assert!(wired(&root.join(".claude/skills/release"), &root.join(".agents/skills/release")));
  // Relative on unix, so a moved repository keeps its links (a Windows junction is always absolute)
  #[cfg(unix)]
  assert_eq!(std::fs::read_link(root.join(".claude/skills/release")).unwrap(), PathBuf::from("../../.agents/skills/release"));
  assert_eq!(std::fs::read_to_string(root.join(".git/info/exclude")).unwrap(), "/.claude/skills/release\n");
  assert!(v.project_auto && v.project_linked && !v.user_linked && !v.auto);
  // User level waits for the panel, which lists the Claude skill link, the three prompt files and nothing native
  assert!(std::fs::symlink_metadata(home.join(".claude/skills/dig")).is_err());
  let plan = |v: &SharedView| v.plan.iter().map(|x| (x.agent.clone(), x.name.clone(), x.state)).collect::<Vec<_>>();
  assert_eq!(
    plan(&v),
    [
      ("claude".into(), "dig".into(), ReachState::Missing),
      ("claude".into(), "CLAUDE.md".into(), ReachState::Missing),
      ("codex".into(), "AGENTS.md".into(), ReachState::Missing),
      ("grok".into(), "AGENTS.md".into(), ReachState::Conflict),
    ]
  );

  // Link the skill and Codex, keep the shared prompt over Grok's, skip Claude's prompt
  let picks = vec![
    pick(&home.join(".claude/skills/dig"), Choice::Link),
    pick(&home.join(".codex/AGENTS.md"), Choice::Link),
    pick(&home.join(".grok/AGENTS.md"), Choice::KeepShared),
    pick(&home.join(".claude/CLAUDE.md"), Choice::Skip),
  ];
  cfg.apply(SharedAction::Link { picks, auto: true }, &p, &agents).await.unwrap();
  let shared = home.join(".agents/AGENTS.md");
  assert!(wired(&home.join(".claude/skills/dig"), &home.join(".agents/skills/dig")));
  assert!(wired(&home.join(".codex/AGENTS.md"), &shared) && wired(&home.join(".grok/AGENTS.md"), &shared));
  assert!(!home.join(".claude/CLAUDE.md").exists());
  let v = cfg.view(p.clone(), agents.clone(), caps()).await;
  assert!(v.auto && v.user_linked);
  assert_eq!(plan(&v), [("claude".into(), "CLAUDE.md".into(), ReachState::Missing)]);
  assert!(v.plan[0].skipped);

  // Auto: a new skill is linked on the next look, the skipped prompt stays untouched
  std::fs::create_dir_all(home.join(".agents/skills/new")).unwrap();
  std::fs::write(home.join(".agents/skills/new/SKILL.md"), "x").unwrap();
  cfg.view(p.clone(), agents.clone(), caps()).await;
  assert!(wired(&home.join(".claude/skills/new"), &home.join(".agents/skills/new")));
  assert!(!home.join(".claude/CLAUDE.md").exists());
  // A link to a deleted skill goes away
  std::fs::remove_dir_all(home.join(".agents/skills/new")).unwrap();
  cfg.maintain(&p, &agents).await.unwrap();
  assert!(std::fs::symlink_metadata(home.join(".claude/skills/new")).is_err());

  // Sharing the project with the team takes its links out of info/exclude, and back
  cfg.apply(SharedAction::ShareProject { share: true }, &p, &agents).await.unwrap();
  assert_eq!(std::fs::read_to_string(root.join(".git/info/exclude")).unwrap(), "");
  assert!(cfg.view(p.clone(), agents.clone(), caps()).await.project_shared);
  cfg.apply(SharedAction::ShareProject { share: false }, &p, &agents).await.unwrap();
  assert_eq!(std::fs::read_to_string(root.join(".git/info/exclude")).unwrap(), "/.claude/skills/release\n");

  // Undo is user level only; the backed-up Grok file comes back byte for byte
  cfg.apply(SharedAction::Unlink, &p, &agents).await.unwrap();
  assert!(std::fs::symlink_metadata(home.join(".claude/skills/dig")).is_err());
  assert_eq!(std::fs::read_to_string(home.join(".grok/AGENTS.md")).unwrap(), "# mine\n");
  assert_eq!(std::fs::read_to_string(home.join(".codex/AGENTS.md")).unwrap(), "# shared\n");
  assert!(!wired(&home.join(".codex/AGENTS.md"), &shared));
  assert!(wired(&root.join(".claude/skills/release"), &root.join(".agents/skills/release")));
  assert!(!cfg.view(p.clone(), agents.clone(), caps()).await.auto);

  // Project level off removes the project links and keeps them away
  cfg.apply(SharedAction::ProjectAuto { on: false }, &p, &agents).await.unwrap();
  assert!(std::fs::symlink_metadata(root.join(".claude/skills/release")).is_err());
  assert_eq!(std::fs::read_to_string(root.join(".git/info/exclude")).unwrap(), "");
  let v = cfg.view(p.clone(), agents.clone(), caps()).await;
  assert!(!v.project_auto && !v.project_linked);
  assert!(std::fs::symlink_metadata(root.join(".claude/skills/release")).is_err());
}

#[tokio::test]
async fn resolves_private_skills_and_prompts() {
  let (t, p, cfg, agents) = setup();
  let home = p.home.clone();
  // A Claude-only skill, a differing Claude copy at a link point and an identical Codex duplicate
  for (dir, body) in [(".claude/skills/solo", "solo"), (".claude/skills/dig", "changed"), (".codex/skills/dig", "---\nname: dig\ndescription: d\n---\n")] {
    std::fs::create_dir_all(home.join(dir)).unwrap();
    std::fs::write(home.join(dir).join("SKILL.md"), body).unwrap();
  }
  let v = cfg.view(p.clone(), agents.clone(), caps()).await;
  let m = |n: &str, a: &str| v.private_skills.iter().find(|s| s.name == n && s.agent == a).map(|s| s.matches);
  // Claude's differing dig sits where the link goes, so the panel owns it rather than the private list
  assert_eq!((m("solo", "claude"), m("dig", "claude"), m("dig", "codex")), (Some(PrivateMatch::Unique), None, Some(PrivateMatch::Same)));
  assert!(v.plan.iter().any(|x| x.name == "dig" && x.state == ReachState::Conflict));

  let path = |d: &str| s(&home.join(d));
  cfg.apply(SharedAction::ResolveSkill { path: path(".claude/skills/solo"), keep: Keep::Private }, &p, &agents).await.unwrap();
  assert!(home.join(".agents/skills/solo/SKILL.md").exists());
  assert!(wired(&home.join(".claude/skills/solo"), &home.join(".agents/skills/solo")));
  cfg.apply(SharedAction::Link { picks: vec![pick(&home.join(".claude/skills/dig"), Choice::KeepPrivate)], auto: false }, &p, &agents).await.unwrap();
  assert_eq!(std::fs::read_to_string(home.join(".agents/skills/dig/SKILL.md")).unwrap(), "changed");
  assert!(wired(&home.join(".claude/skills/dig"), &home.join(".agents/skills/dig")));
  cfg.apply(SharedAction::ResolveSkill { path: path(".codex/skills/dig"), keep: Keep::Shared }, &p, &agents).await.unwrap();
  assert!(!home.join(".codex/skills/dig").exists());
  assert!(cfg.view(p.clone(), agents.clone(), caps()).await.private_skills.is_empty());

  // Deleting a shared skill backs it up and prunes the agents' links to it; a private one goes the same way
  std::fs::create_dir_all(home.join(".codex/skills/own")).unwrap();
  std::fs::write(home.join(".codex/skills/own/SKILL.md"), "own").unwrap();
  cfg.apply(SharedAction::RemoveSkill { path: path(".agents/skills/solo") }, &p, &agents).await.unwrap();
  assert!(!home.join(".agents/skills/solo").exists());
  assert!(std::fs::symlink_metadata(home.join(".claude/skills/solo")).is_err());
  cfg.apply(SharedAction::RemoveSkill { path: path(".codex/skills/own") }, &p, &agents).await.unwrap();
  assert!(!home.join(".codex/skills/own").exists());
  assert!(cfg.apply(SharedAction::RemoveSkill { path: path(".agents") }, &p, &agents).await.is_err());
  let backups = t.path().join("data/backups/shared");
  let kept: Vec<_> = std::fs::read_dir(&backups).unwrap().flatten().flat_map(|d| std::fs::read_dir(d.path()).unwrap().flatten()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
  assert!(kept.iter().any(|n| n.ends_with(".agents_skills_solo")) && kept.iter().any(|n| n.ends_with(".codex_skills_own")));

  // No shared prompt yet: Grok's own file becomes it and Claude's differing one imports it, in one submit
  std::fs::write(home.join(".grok/AGENTS.md"), "# grok rules\n").unwrap();
  std::fs::write(home.join(".claude/CLAUDE.md"), "# claude rules\n").unwrap();
  let v = cfg.view(p.clone(), agents.clone(), caps()).await;
  assert!(!v.shared_prompt);
  let picks = vec![pick(&home.join(".claude/CLAUDE.md"), Choice::KeepShared), pick(&home.join(".grok/AGENTS.md"), Choice::KeepPrivate)];
  cfg.apply(SharedAction::Link { picks, auto: false }, &p, &agents).await.unwrap();
  assert_eq!(std::fs::read_to_string(home.join(".agents/AGENTS.md")).unwrap(), "# grok rules\n");
  assert!(wired(&home.join(".grok/AGENTS.md"), &home.join(".agents/AGENTS.md")));
  assert_eq!(std::fs::read_to_string(home.join(".claude/CLAUDE.md")).unwrap(), "@~/.agents/AGENTS.md\n");
  // Undo puts Claude's own file back
  cfg.apply(SharedAction::Unlink, &p, &agents).await.unwrap();
  assert_eq!(std::fs::read_to_string(home.join(".claude/CLAUDE.md")).unwrap(), "# claude rules\n");
}

// Antigravity reads a project's .agents/skills but not ~/.agents/skills: adopting its project skill leaves no link
// behind in .gemini/skills, adopting a global one links it back into ~/.gemini/config/skills
#[tokio::test]
async fn an_adopted_antigravity_skill_is_linked_back_only_where_it_does_not_read_agents() {
  let (_t, p, cfg, _) = setup();
  let agents = vec!["antigravity".to_owned()];
  let (home, root) = (p.home.clone(), p.root.clone().unwrap());
  for dir in [root.join(".gemini/skills/lint"), home.join(".gemini/config/skills/notes")] {
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), "---\nname: x\n---\n").unwrap();
  }
  cfg.apply(SharedAction::ResolveSkill { path: s(&root.join(".gemini/skills/lint")), keep: Keep::Private }, &p, &agents).await.unwrap();
  assert!(root.join(".agents/skills/lint/SKILL.md").exists());
  assert!(std::fs::symlink_metadata(root.join(".gemini/skills/lint")).is_err());
  cfg.apply(SharedAction::ResolveSkill { path: s(&home.join(".gemini/config/skills/notes")), keep: Keep::Private }, &p, &agents).await.unwrap();
  assert!(home.join(".agents/skills/notes/SKILL.md").exists());
  assert!(wired(&home.join(".gemini/config/skills/notes"), &home.join(".agents/skills/notes")));
}

#[tokio::test]
async fn undo_restores_a_private_skill_and_spares_a_repointed_link() {
  let (_t, p, cfg, agents) = setup();
  let home = p.home.clone();
  // Claude's own `dig` differs from the shared one; keeping the shared one sets it aside behind a link
  std::fs::create_dir_all(home.join(".claude/skills/dig")).unwrap();
  std::fs::write(home.join(".claude/skills/dig/SKILL.md"), "mine").unwrap();
  let solo = home.join(".claude/skills/solo");
  std::fs::create_dir_all(&solo).unwrap();
  std::fs::write(solo.join("SKILL.md"), "solo").unwrap();
  let path = |d: &str| s(&home.join(d));
  cfg.apply(SharedAction::ResolveSkill { path: path(".claude/skills/dig"), keep: Keep::Shared }, &p, &agents).await.unwrap();
  assert!(wired(&home.join(".claude/skills/dig"), &home.join(".agents/skills/dig")));
  cfg.apply(SharedAction::ResolveSkill { path: path(".claude/skills/solo"), keep: Keep::Private }, &p, &agents).await.unwrap();
  assert!(wired(&solo, &home.join(".agents/skills/solo")));
  // The user points the solo link at a skill of their own
  let custom = home.join("custom-solo");
  std::fs::create_dir_all(&custom).unwrap();
  crate::platform::files::remove_link(&solo).unwrap();
  crate::platform::files::create_link(&solo, &custom, &custom).unwrap();

  cfg.apply(SharedAction::Unlink, &p, &agents).await.unwrap();
  assert!(!links::is_link(&home.join(".claude/skills/dig")));
  assert_eq!(std::fs::read_to_string(home.join(".claude/skills/dig/SKILL.md")).unwrap(), "mine");
  assert!(wired(&solo, &custom));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_mcp_adds_all_land() {
  let (_t, p, cfg, agents) = setup();
  let mut tasks = tokio::task::JoinSet::new();
  for i in 0..16 {
    let (p, cfg, agents) = (p.clone(), cfg.clone(), agents.clone());
    tasks.spawn(async move {
      let json = format!(r#"{{ "command": "srv{i}" }}"#);
      cfg.apply(SharedAction::AddMcp { scope: SharedScope::Global, json, name: Some(format!("s{i}")) }, &p, &agents).await.unwrap();
    });
  }
  while let Some(r) = tasks.join_next().await {
    r.unwrap();
  }
  let text = std::fs::read_to_string(p.mcp_file(SharedScope::Global).unwrap()).unwrap();
  let data: serde_json::Value = serde_json::from_str(&text).unwrap();
  assert_eq!(data["mcpServers"].as_object().unwrap().len(), 16);
}

#[tokio::test]
async fn a_turned_off_agent_loses_its_links_and_gets_them_back() {
  let (_t, p, cfg, agents) = setup();
  let home = p.home.clone();
  let root = p.root.clone().unwrap();
  std::fs::write(home.join(".agents/AGENTS.md"), "# shared\n").unwrap();
  std::fs::write(home.join(".codex/AGENTS.md"), "# shared\n").unwrap();
  let (dig, release, codex, shared) = (home.join(".claude/skills/dig"), root.join(".claude/skills/release"), home.join(".codex/AGENTS.md"), home.join(".agents/AGENTS.md"));
  let dig_ok = || wired(&dig, &home.join(".agents/skills/dig"));
  let release_ok = || wired(&release, &root.join(".agents/skills/release"));
  let picks = vec![pick(&home.join(".claude/skills/dig"), Choice::Link), pick(&home.join(".codex/AGENTS.md"), Choice::Link)];
  cfg.view(p.clone(), agents.clone(), caps()).await;
  cfg.apply(SharedAction::Link { picks, auto: true }, &p, &agents).await.unwrap();
  assert!(release_ok() && dig_ok());

  // Claude off: its user and project links go, Codex's stays
  cfg.retire(&["claude".into()]).await;
  let on: Vec<String> = agents.iter().filter(|a| *a != "claude").cloned().collect();
  let v = cfg.view(p.clone(), on, caps()).await;
  assert!(std::fs::symlink_metadata(home.join(".claude/skills/dig")).is_err());
  assert!(std::fs::symlink_metadata(root.join(".claude/skills/release")).is_err());
  assert!(wired(&codex, &shared));
  assert!(!v.plan.iter().any(|x| x.agent == "claude"));
  // The identical Codex copy that was moved aside comes back when Codex is turned off
  cfg.retire(&["codex".into()]).await;
  assert_eq!(std::fs::read_to_string(home.join(".codex/AGENTS.md")).unwrap(), "# shared\n");
  assert!(!wired(&codex, &shared));

  // Back on: auto links them again
  cfg.view(p.clone(), agents.clone(), caps()).await;
  assert!(dig_ok() && release_ok());
  assert!(wired(&codex, &shared));
}

#[tokio::test]
async fn pi_sees_project_skills_once_trusted() {
  // The env var would point the write at a real Pi store
  if std::env::var_os("PI_CODING_AGENT_DIR").is_some() {
    return;
  }
  let (_t, p, cfg, mut agents) = setup();
  agents.push("pi".into());
  let pi_reach = |v: &SharedView| {
    let skill = v.skills.iter().find(|x| x.name == "release").unwrap();
    skill.reach.iter().find(|r| r.agent == "pi").map(|r| r.state)
  };
  let v = cfg.view(p.clone(), agents.clone(), caps()).await;
  assert!(v.pi_untrusted);
  assert_eq!(pi_reach(&v), Some(ReachState::Untrusted));
  cfg.apply(SharedAction::TrustPi, &p, &agents).await.unwrap();
  let v = cfg.view(p.clone(), agents.clone(), caps()).await;
  assert!(!v.pi_untrusted);
  assert_eq!(pi_reach(&v), Some(ReachState::Native));
  // Global skills never need trust
  let dig = v.skills.iter().find(|x| x.name == "dig").unwrap();
  assert!(dig.reach.iter().any(|r| r.agent == "pi" && r.state == ReachState::Native));
}

#[tokio::test]
async fn overwrite_wires_every_agent_and_turning_it_off_restores_them() {
  let (_t, p, cfg, agents) = setup();
  let home = p.home.clone();
  let (codex, claude, grok) = (home.join(".codex/AGENTS.md"), home.join(".claude/CLAUDE.md"), home.join(".grok/AGENTS.md"));
  let shared = home.join(".agents/AGENTS.md");
  let codex_own = "# 全局指令\n\n- 称呼：老板\n\n## 提交风格\n\n默认中文\n";
  let claude_own = "# 全局指令\n\n- 称呼：老板\n\n## Git\n\n- 不加水印\n```\nx\n```\n";
  std::fs::write(&codex, codex_own).unwrap();
  std::fs::write(&claude, claude_own).unwrap();
  // Claude's differing dig sits at a link point; Claude and Codex have skills of their own, Codex a differing dig too
  for (dir, body) in [(".claude/skills/dig", "mine"), (".claude/skills/only", "claude only"), (".codex/skills/solo", "codex solo"), (".codex/skills/dig", "codex dig")] {
    std::fs::create_dir_all(home.join(dir)).unwrap();
    std::fs::write(home.join(dir).join("SKILL.md"), body).unwrap();
  }
  // Grok's prompt was skipped in the link panel once
  cfg.apply(SharedAction::Link { picks: vec![pick(&grok, Choice::Skip)], auto: false }, &p, &agents).await.unwrap();

  // Without a shared prompt every line of the agents' own files is theirs alone
  let v = cfg.view(p.clone(), agents.clone(), caps()).await;
  let unique = |v: &SharedView, f: &Path| v.takeover.iter().find(|x| Path::new(&x.path) == f).map(|x| x.unique.clone());
  assert_eq!(unique(&v, &codex).as_deref(), Some(codex_own.trim_end()));
  assert!(unique(&v, &claude).is_some());

  // Merging both: Claude adds only what Codex lacks, fences kept; then everything is wired, the skipped point too
  let merge = vec![s(&codex), s(&claude)];
  cfg.apply(SharedAction::Overwrite { on: true, merge }, &p, &agents).await.unwrap();
  assert_eq!(std::fs::read_to_string(&shared).unwrap(), "# 全局指令\n\n- 称呼：老板\n\n## 提交风格\n\n默认中文\n\n## Git\n\n- 不加水印\n```\nx\n```\n");
  assert!(wired(&codex, &shared) && wired(&grok, &shared));
  assert_eq!(std::fs::read_to_string(&claude).unwrap(), "@~/.agents/AGENTS.md\n");
  assert!(wired(&home.join(".claude/skills/dig"), &home.join(".agents/skills/dig")));
  // The agents' own skills join the shared folder (Claude gets its one back as a link); Codex's differing dig is set aside
  assert_eq!(std::fs::read_to_string(home.join(".agents/skills/only/SKILL.md")).unwrap(), "claude only");
  assert_eq!(std::fs::read_to_string(home.join(".agents/skills/solo/SKILL.md")).unwrap(), "codex solo");
  assert!(wired(&home.join(".claude/skills/only"), &home.join(".agents/skills/only")) && wired(&home.join(".claude/skills/solo"), &home.join(".agents/skills/solo")));
  assert!(std::fs::symlink_metadata(home.join(".codex/skills/solo")).is_err() && std::fs::symlink_metadata(home.join(".codex/skills/dig")).is_err());
  let v = cfg.view(p.clone(), agents.clone(), caps()).await;
  assert!(v.overwrite && v.takeover.is_empty() && v.plan.is_empty());
  assert!(!v.private_skills.iter().any(|x| x.scope == SharedScope::Global));

  // A link replaced by a file is wired again on the next look
  std::fs::remove_file(&codex).unwrap();
  std::fs::write(&codex, "# drift\n").unwrap();
  cfg.view(p.clone(), agents.clone(), caps()).await;
  assert!(wired(&codex, &shared));

  // The page's editor writes through to every agent; an edit started from an older text is refused
  let base = std::fs::read_to_string(&shared).unwrap();
  cfg.apply(SharedAction::SavePrompt { scope: SharedScope::Global, text: "# new\n".into(), base: base.clone() }, &p, &agents).await.unwrap();
  assert_eq!(std::fs::read_to_string(&codex).unwrap(), "# new\n");
  assert!(cfg.apply(SharedAction::SavePrompt { scope: SharedScope::Global, text: "# lost\n".into(), base }, &p, &agents).await.is_err());
  assert_eq!(std::fs::read_to_string(&shared).unwrap(), "# new\n");

  // Off: every agent gets its own file back (the first one, not the drift); the shared prompt stays
  cfg.apply(SharedAction::Overwrite { on: false, merge: vec![] }, &p, &agents).await.unwrap();
  assert_eq!(std::fs::read_to_string(&codex).unwrap(), codex_own);
  assert_eq!(std::fs::read_to_string(&claude).unwrap(), claude_own);
  assert_eq!(std::fs::read_to_string(home.join(".claude/skills/dig/SKILL.md")).unwrap(), "mine");
  for (dir, body) in [(".claude/skills/only", "claude only"), (".codex/skills/solo", "codex solo"), (".codex/skills/dig", "codex dig")] {
    assert!(!links::is_link(&home.join(dir)));
    assert_eq!(std::fs::read_to_string(home.join(dir).join("SKILL.md")).unwrap(), body);
  }
  assert!(!home.join(".agents/skills/only").exists() && !home.join(".agents/skills/solo").exists());
  assert!(std::fs::symlink_metadata(&grok).is_err());
  assert_eq!(std::fs::read_to_string(&shared).unwrap(), "# new\n");
  assert!(!cfg.view(p.clone(), agents.clone(), caps()).await.overwrite);
  assert!(!cfg.ledger.read().await.entries.iter().any(|e| e.overwrite));
}

#[tokio::test]
async fn overwrite_takes_over_mixed_and_unreadable_files_and_rewired_panel_links() {
  let (_t, p, cfg, agents) = setup();
  let home = p.home.clone();
  let (claude, codex, grok) = (home.join(".claude/CLAUDE.md"), home.join(".codex/AGENTS.md"), home.join(".grok/AGENTS.md"));
  std::fs::write(home.join(".agents/AGENTS.md"), "# shared\n").unwrap();
  // Claude imports the shared prompt but adds rules of its own; Codex's file is not UTF-8
  let mixed = "@~/.agents/AGENTS.md\n\n- claude only rule\n";
  std::fs::write(&claude, mixed).unwrap();
  std::fs::write(&codex, b"\xff\xfe rules").unwrap();
  // Grok was linked by the panel, then replaced by a file of the user's own
  std::fs::write(&grok, "# shared\n").unwrap();
  cfg.apply(SharedAction::Link { picks: vec![pick(&grok, Choice::Link)], auto: false }, &p, &agents).await.unwrap();
  std::fs::remove_file(&grok).unwrap();
  std::fs::write(&grok, "# grok own\n").unwrap();

  let v = cfg.view(p.clone(), agents.clone(), caps()).await;
  let claude_take = v.takeover.iter().find(|x| Path::new(&x.path) == claude).unwrap();
  assert_eq!(claude_take.unique, "- claude only rule");
  cfg.apply(SharedAction::Overwrite { on: true, merge: vec![] }, &p, &agents).await.unwrap();
  assert_eq!(std::fs::read_to_string(&claude).unwrap(), "@~/.agents/AGENTS.md\n");
  let shared = home.join(".agents/AGENTS.md");
  assert!(wired(&codex, &shared) && wired(&grok, &shared));

  // Off: each file comes back as it was right before overwrite, the unreadable one byte for byte
  cfg.apply(SharedAction::Overwrite { on: false, merge: vec![] }, &p, &agents).await.unwrap();
  assert_eq!(std::fs::read_to_string(&claude).unwrap(), mixed);
  assert_eq!(std::fs::read(&codex).unwrap(), b"\xff\xfe rules");
  assert_eq!(std::fs::read_to_string(&grok).unwrap(), "# grok own\n");
  // Saving over a prompt that cannot be read is refused rather than taken for an empty file
  std::fs::write(home.join(".agents/AGENTS.md"), b"\xff").unwrap();
  assert!(cfg.apply(SharedAction::SavePrompt { scope: SharedScope::Global, text: "x".into(), base: String::new() }, &p, &agents).await.is_err());
}

#[tokio::test]
async fn a_deleted_shared_skill_keeps_the_private_backup_on_record() {
  let (_t, p, cfg, agents) = setup();
  let home = p.home.clone();
  std::fs::create_dir_all(home.join(".claude/skills/dig")).unwrap();
  std::fs::write(home.join(".claude/skills/dig/SKILL.md"), "mine").unwrap();
  cfg.apply(SharedAction::Overwrite { on: true, merge: vec![] }, &p, &agents).await.unwrap();
  assert!(wired(&home.join(".claude/skills/dig"), &home.join(".agents/skills/dig")));
  cfg.apply(SharedAction::RemoveSkill { path: s(&home.join(".agents/skills/dig")) }, &p, &agents).await.unwrap();
  assert!(std::fs::symlink_metadata(home.join(".claude/skills/dig")).is_err());
  cfg.apply(SharedAction::Overwrite { on: false, merge: vec![] }, &p, &agents).await.unwrap();
  assert_eq!(std::fs::read_to_string(home.join(".claude/skills/dig/SKILL.md")).unwrap(), "mine");
}

#[test]
fn unique_lines_keep_order_paragraphs_and_fences() {
  let base = "# A\n\n- one\n```\n";
  assert_eq!(view::unique_lines("# A\n- one\n- two\n\n@~/.agents/AGENTS.md\n\n```\ncode\n```\n", base, &[CLAUDE_GLOBAL_IMPORT.to_owned()]), "- two\n\n```\ncode\n```");
  assert_eq!(view::unique_lines("# A\n\n- one\n", base, &[]), "");
  // A fence with a language tag the shared prompt also has is kept, or the block would lose its opening
  assert_eq!(view::unique_lines("~~~sh\necho new\n~~~\n", "~~~sh\necho old\n~~~\n", &[]), "~~~sh\necho new\n~~~");
  // A known table leaves no separator row behind; a fenced block with a blank line inside still counts as one paragraph
  let table = "| a | b |\n|---|---|\n| x | y |\n";
  assert_eq!(view::unique_lines(&format!("{table}\n---\n\n- new\n"), table, &[]), "- new");
  assert_eq!(view::unique_lines("```\nold\n\nnew\n```\n", "old\n", &[]), "```\n\nnew\n```");
}

#[tokio::test]
async fn project_claude_md_gets_an_import() {
  let (_t, p, cfg, agents) = setup();
  let root = p.root.clone().unwrap();
  std::fs::write(root.join("AGENTS.md"), "# project\n").unwrap();
  std::fs::write(root.join("CLAUDE.md"), "# claude only\n").unwrap();
  let v = cfg.view(p.clone(), agents.clone(), caps()).await;
  let project = v.prompts.iter().find(|x| x.scope == SharedScope::Project).unwrap();
  assert!(project.reach.iter().any(|r| r.agent == "claude" && r.state == ReachState::Conflict));
  cfg.apply(SharedAction::ClaudeImport, &p, &agents).await.unwrap();
  assert_eq!(std::fs::read_to_string(root.join("CLAUDE.md")).unwrap(), "@AGENTS.md\n\n# claude only\n");
  let v = cfg.view(p.clone(), agents.clone(), caps()).await;
  let project = v.prompts.iter().find(|x| x.scope == SharedScope::Project).unwrap();
  assert!(project.reach.iter().any(|r| r.agent == "claude" && r.state == ReachState::Native));
  // A project-level entry: the user-level undo leaves it alone
  cfg.apply(SharedAction::Unlink, &p, &agents).await.unwrap();
  assert_eq!(std::fs::read_to_string(root.join("CLAUDE.md")).unwrap(), "@AGENTS.md\n\n# claude only\n");
}
