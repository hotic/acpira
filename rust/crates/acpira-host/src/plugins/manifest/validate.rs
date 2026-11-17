//! Reading and checking a manifest: schema, names, menus, runtime commands, dependencies, packages and the files it points at, into `Findings`

use std::path::Path;

use serde_json::Value;

use super::{
  BACKDROP_MAX_BYTES, BUTTON_ICONS, BUTTON_LOOKS, BUTTON_MAX_CHARS, CATCH_UP, CONTRIBUTION_KEYS, CSS_MAX_BYTES,
  CommandDef, FILE, Findings, GLASS, KEYS, LAYERS, MENU_DESCRIPTION_MAX_CHARS, MENU_KINDS, MENU_LABEL_MAX_CHARS,
  MENU_MAX_ITEMS, Manifest, NAME_MAX, RUNTIME_EVENTS, RUNTIME_HOOKS, RUNTIME_TIMEOUT_SECS, SCENE_MAX_BYTES,
  SCHEDULE_TIMEOUT_MAX_SECS, SCHEMA, SCOPE_AGENT, THEME_BASES, THEME_TOKENS, TOOL_TIMEOUT_SECS, UI_HTML_MAX_BYTES,
  UI_SIZE_MAX, provider_of, slot_box, valid_command_name, valid_id, valid_method_name, valid_tool_name,
};

/// Read `plugin.json` from `dir`: the manifest, or why it cannot be one
pub fn read(dir: &Path) -> Result<(Manifest, Findings), String> {
  let path = dir.join(FILE);
  let bytes = std::fs::read(&path).map_err(|e| format!("{FILE}: {e}"))?;
  parse(&bytes)
}

pub fn parse(bytes: &[u8]) -> Result<(Manifest, Findings), String> {
  let raw: Value = serde_json::from_slice(bytes).map_err(|e| format!("{FILE}: invalid JSON at line {}, column {}", e.line(), e.column()))?;
  let Some(obj) = raw.as_object() else { return Err(format!("{FILE}: expected an object")) };
  let schema = obj.get("schema").and_then(Value::as_u64);
  if schema != Some(SCHEMA as u64) {
    return Err(format!("{FILE}: schema {} is not one this Acpira reads (it reads {SCHEMA})", obj.get("schema").map(Value::to_string).unwrap_or_else(|| "missing".into())));
  }
  let mut findings = Findings::default();
  for k in obj.keys().filter(|k| !KEYS.contains(&k.as_str())) {
    findings.notes.push(format!("unknown key `{k}` ignored"));
  }
  if let Some(c) = obj.get("contributes").and_then(Value::as_object) {
    for k in c.keys().filter(|k| !CONTRIBUTION_KEYS.contains(&k.as_str())) {
      findings.notes.push(format!("unknown contribution `{k}` ignored"));
    }
  }
  let manifest: Manifest = serde_json::from_value(raw).map_err(|e| format!("{FILE}: {e}"))?;
  Ok((manifest, findings))
}

/// A command button's `menu`: picks are stored by group name and toggle id, so those share one namespace, and an
/// action needs a runtime that answers the `menu` hook
fn check_menu(m: &Manifest, c: &CommandDef, at: &str, findings: &mut Findings) {
  let mut err = |s: String| findings.errors.push(s);
  if c.button.is_none() || c.draft || c.input.is_some() {
    err(format!("{at}: a `menu` needs a `button`, and takes neither `draft` nor `input`"));
  }
  if c.menu.len() > MENU_MAX_ITEMS {
    err(format!("{at}: a menu has at most {MENU_MAX_ITEMS} rows"));
  }
  let menu_hooked = m.contributes.runtime.as_ref().is_some_and(|r| r.main.is_some() || r.hooks.iter().any(|h| h == "menu"));
  let mut ids = std::collections::HashSet::new();
  let mut keys = std::collections::HashMap::new();
  let mut defaults = std::collections::HashSet::new();
  for item in &c.menu {
    let row = format!("{at}: menu row `{}`", item.id);
    if !MENU_KINDS.contains(&item.kind.as_str()) {
      err(format!("{row}: kind `{}` is not one of {}", item.kind, MENU_KINDS.join(", ")));
      continue;
    }
    if item.kind == "separator" {
      continue;
    }
    if !valid_command_name(&item.id) {
      err(format!("{row}: the id must be lowercase letters, digits, `-` and `_`, starting with a letter, at most {NAME_MAX} characters"));
    }
    if !ids.insert(item.id.as_str()) {
      err(format!("{row} is declared twice"));
    }
    let label = item.label.trim().chars().count();
    if label == 0 || label > MENU_LABEL_MAX_CHARS {
      err(format!("{row}: `label` is 1 to {MENU_LABEL_MAX_CHARS} characters"));
    }
    if item.description.as_deref().is_some_and(|d| d.chars().count() > MENU_DESCRIPTION_MAX_CHARS) {
      err(format!("{row}: `description` is at most {MENU_DESCRIPTION_MAX_CHARS} characters"));
    }
    match item.kind.as_str() {
      "radio" => match item.group.as_deref() {
        Some(g) if valid_command_name(g) => {
          if keys.insert(g, "radio").is_some_and(|k| k != "radio") {
            err(format!("{row}: group `{g}` is also a toggle's id"));
          }
          if item.default && !defaults.insert(g) {
            err(format!("{row}: group `{g}` has more than one `default`"));
          }
        }
        _ => err(format!("{row}: a radio row names its `group` (lowercase letters, digits, `-` and `_`)")),
      },
      "toggle" if keys.insert(item.id.as_str(), "toggle").is_some() => err(format!("{row}: the id is also a group's name")),
      "action" if !menu_hooked => err(format!("{row}: an action needs a runtime that answers the `menu` hook")),
      _ => {}
    }
    if item.group.is_some() && item.kind != "radio" {
      err(format!("{row}: only a radio row has a `group`"));
    }
    if item.default && item.kind == "action" {
      err(format!("{row}: an action has no `default`"));
    }
  }
}

/// The static check of a manifest against the folder it sits in (the source folder or its frozen copy). `which`
/// resolves a program on PATH
pub fn check(m: &Manifest, folder_id: &str, dir: &Path, which: &dyn Fn(&str) -> bool, findings: &mut Findings) {
  let err = |f: &mut Findings, s: String| f.errors.push(s);
  if !valid_id(&m.id) {
    err(findings, format!("id `{}` must be lowercase letters, digits and `-`, at most {NAME_MAX} characters", m.id));
  }
  if m.id != folder_id {
    err(findings, format!("id `{}` must be the folder's name `{folder_id}`", m.id));
  }
  if m.description.trim().is_empty() {
    findings.notes.push("no description".into());
  }
  if let Some(l) = &m.layer
    && !LAYERS.contains(&l.as_str())
  {
    findings.notes.push(format!("layer `{l}` is not one of {}", LAYERS.join(", ")));
  }
  if m.contributes.kinds().is_empty() {
    err(findings, "contributes nothing".into());
  }
  for r in &m.requires {
    if !which(r) {
      err(findings, format!("requires `{r}`, which is not on PATH"));
    }
  }
  if let Some(check) = &m.check {
    command_exists(check, dir, which, "check", findings);
  }
  let mut seen = std::collections::HashSet::new();
  for t in &m.contributes.tools {
    let at = format!("tool `{}`", t.name);
    if !valid_tool_name(&t.name) {
      err(findings, format!("{at}: the name must be lowercase letters, digits and `_`, starting with a letter, at most {NAME_MAX} characters"));
    }
    if !seen.insert(t.name.as_str()) {
      err(findings, format!("{at} is declared twice"));
    }
    if t.description.trim().is_empty() {
      err(findings, format!("{at}: a description is needed, the model picks tools by it"));
    }
    if t.input.get("type").and_then(Value::as_str) != Some("object") {
      err(findings, format!("{at}: `input` must be a JSON Schema of type object"));
    }
    if t.timeout.is_some_and(|s| s > TOOL_TIMEOUT_SECS) {
      findings.notes.push(format!("{at}: timeout cut to {TOOL_TIMEOUT_SECS} s"));
    }
    if !t.command.is_empty() {
      command_exists(&t.command, dir, which, &at, findings);
    } else if m.contributes.runtime.is_none() {
      err(findings, format!("{at}: needs a `command`, or a runtime whose definition answers it (`tools` in `definePlugin`)"));
    }
  }
  if let Some(css) = &m.contributes.style {
    css_file(dir, css, "style css", findings);
  }
  if let Some(theme) = &m.contributes.theme {
    if let Some(css) = &theme.css {
      css_file(dir, css, "theme css", findings);
    }
    for k in theme.tokens.keys() {
      if !THEME_TOKENS.contains(&k.as_str()) {
        err(findings, format!("theme token `{k}` is not one a theme may set"));
      }
    }
    for (k, v) in &theme.tokens {
      if !v.as_str().is_some_and(safe_css_value) {
        err(findings, format!("theme token `{k}`: the value must be a CSS value without `;`, braces or `<`"));
      }
    }
    if let Some(b) = theme.base.as_deref().filter(|b| !THEME_BASES.contains(b)) {
      err(findings, format!("theme base `{b}` is not one of {}", THEME_BASES.join(", ")));
    }
    if let Some(g) = theme.glass.as_deref().filter(|g| !GLASS.contains(g)) {
      err(findings, format!("theme glass `{g}` is not one of {}", GLASS.join(", ")));
    }
    for (what, file, max) in [("backdrop", &theme.backdrop, BACKDROP_MAX_BYTES), ("scene", &theme.scene, SCENE_MAX_BYTES)] {
      let Some(b) = file else { continue };
      match inside(dir, b).and_then(|p| std::fs::metadata(p).ok()).filter(std::fs::Metadata::is_file) {
        None => err(findings, format!("theme {what} `{b}` not found in the plugin")),
        Some(meta) if meta.len() > max => err(findings, format!("theme {what} `{b}` is over {} KB", max / 1024)),
        Some(_) => {}
      }
    }
    if theme.backdrop.is_some() && theme.scene.is_some() {
      err(findings, "theme: `backdrop` and `scene` exclude each other (a scene is drawn by Acpira's renderer, a backdrop draws itself)".into());
    }
    if theme.backdrop.is_some() || theme.scene.is_some() {
      if theme.base.is_none() {
        findings.notes.push("theme: a backdrop without `base` follows the host's light or dark scheme; its tokens must suit both".into());
      }
    } else if theme.glass.is_some() {
      findings.notes.push("theme: `glass` only applies with a `backdrop` or a `scene`".into());
    }
  }
  if let Some(p) = &m.contributes.prompts
    && !inside(dir, p).is_some_and(|d| d.is_dir())
  {
    err(findings, format!("prompts folder `{p}` not found in the plugin"));
  }
  if let Some(p) = &m.contributes.skills
    && !inside(dir, p).is_some_and(|d| d.is_dir())
  {
    err(findings, format!("skills folder `{p}` not found in the plugin"));
  }
  let mut names = std::collections::HashSet::new();
  // A `main` definition answers its commands in code (`commands` of `definePlugin`) without listing the hook
  let hooked = m.contributes.runtime.as_ref().is_some_and(|r| r.main.is_some() || r.hooks.iter().any(|h| h == "command"));
  for c in &m.contributes.commands {
    let at = format!("command `/{}`", c.name);
    if !valid_command_name(&c.name) {
      err(findings, format!("{at}: the name must be lowercase letters, digits, `-` and `_`, starting with a letter, at most {NAME_MAX} characters"));
    }
    if !names.insert(c.name.as_str()) {
      err(findings, format!("{at} is declared twice"));
    }
    // A menu button is the command's way in: typing `/name` then needs the prompt or hook too, the button does not
    if c.menu.is_empty() && c.prompt.as_deref().is_none_or(|p| p.trim().is_empty()) && !hooked {
      err(findings, format!("{at}: needs a `prompt`, or a runtime that answers the `command` hook"));
    }
    if !c.menu.is_empty() {
      check_menu(m, c, &at, findings);
    }
    match c.button.as_deref().map(str::trim) {
      Some(b) if b.is_empty() || b.chars().count() > BUTTON_MAX_CHARS => err(findings, format!("{at}: `button` is the label, 1 to {BUTTON_MAX_CHARS} characters")),
      None if c.draft || c.icon.is_some() || c.look.is_some() || c.input.is_some() => err(findings, format!("{at}: `draft`, `icon`, `look` and `input` need a `button`")),
      _ => {}
    }
    if let Some(i) = c.icon.as_deref().filter(|i| !BUTTON_ICONS.contains(i)) {
      err(findings, format!("{at}: icon `{i}` is not one of {}", BUTTON_ICONS.join(", ")));
    }
    if let Some(l) = c.look.as_deref().filter(|l| !BUTTON_LOOKS.contains(l)) {
      err(findings, format!("{at}: look `{l}` is not one of {}", BUTTON_LOOKS.join(", ")));
    }
    if let Some(i) = c.input.as_deref().filter(|i| !["draft", "none"].contains(i)) {
      err(findings, format!("{at}: input `{i}` is not `draft` or `none`"));
    }
  }
  let mut pieces = std::collections::HashSet::new();
  for u in &m.contributes.ui {
    let at = format!("ui `{}`", u.id);
    if !valid_command_name(&u.id) {
      err(findings, format!("{at}: the id must be lowercase letters, digits, `-` and `_`, starting with a letter, at most {NAME_MAX} characters"));
    }
    if !pieces.insert(u.id.as_str()) {
      err(findings, format!("{at} is declared twice"));
    }
    if !acpira_shared::plugins::UI_SLOTS.contains(&u.slot.as_str()) {
      err(findings, format!("{at}: slot `{}` is not one of {}", u.slot, acpira_shared::plugins::UI_SLOTS.join(", ")));
    }
    if acpira_shared::plugins::BUTTON_SLOTS.contains(&u.slot.as_str()) {
      // A button under each reply or prompt, or over a text selection: one frame per message would cost a document per turn, and a
      // frame cannot read the page's selection
      let slot = &u.slot;
      match u.method.as_deref() {
        None => err(findings, format!("{at}: a `{slot}` piece names the runtime `method` its button calls, instead of `html`")),
        Some(method) if !m.contributes.runtime.as_ref().is_some_and(|r| r.methods.iter().any(|x| x == method)) => {
          err(findings, format!("{at}: method `{method}` is not one of the runtime's `methods`"))
        }
        Some(_) => {}
      }
      if !u.html.is_empty() || u.entry.is_some() {
        err(findings, format!("{at}: a `{slot}` piece is a button, not a document; drop `html` and `entry`"));
      }
      if u.title.trim().is_empty() {
        err(findings, format!("{at}: the button needs a `title`"));
      }
      continue;
    }
    if u.method.is_some() {
      err(findings, format!("{at}: only a `message.actions`, `prompt.actions` or `selection.actions` piece calls a `method`"));
    }
    match (u.html.is_empty(), u.entry.as_deref()) {
      (false, Some(_)) => err(findings, format!("{at}: `html` and `entry` exclude each other")),
      (true, None) => err(findings, format!("{at}: needs an `html` file or a component `entry`")),
      (false, None) => match inside(dir, &u.html).and_then(|p| std::fs::metadata(p).ok()).filter(std::fs::Metadata::is_file) {
        None => err(findings, format!("{at}: html file `{}` not found in the plugin", u.html)),
        Some(meta) if meta.len() > UI_HTML_MAX_BYTES => err(findings, format!("{at}: `{}` is over {} KB", u.html, UI_HTML_MAX_BYTES / 1024)),
        Some(_) => {}
      },
      (true, Some(entry)) => source_file(dir, entry, &format!("{at}: entry"), findings),
    }
    for (name, v) in [("width", u.width), ("height", u.height)] {
      if v.is_some_and(|v| v == 0 || v > UI_SIZE_MAX) {
        err(findings, format!("{at}: {name} must be 1 to {UI_SIZE_MAX} CSS pixels"));
      }
    }
    let (w, h) = slot_box(&u.slot);
    let missing: Vec<&str> = [(w && u.width.is_none(), "`width`"), (h && u.height.is_none(), "`height`")].into_iter().filter(|(m, _)| *m).map(|(_, n)| n).collect();
    if !missing.is_empty() {
      err(findings, format!("{at}: the slot `{}` needs {}", u.slot, missing.join(" and ")));
    }
  }
  let mut sources = std::collections::HashSet::new();
  for raw in &m.contributes.providers {
    match provider_of(raw, &m.id) {
      Err(e) => err(findings, e),
      Ok(p) if !sources.insert(p.id.clone()) => err(findings, format!("provider `{}` is declared twice", p.id)),
      Ok(_) => {}
    }
  }
  if let Some(agent) = &m.contributes.agent {
    if agent.get("builtin").is_some() {
      // A specialised agent runs Acpira's own loop: the agent reads the profile itself, so it is checked the same way
      if agent.get("command").is_some() {
        err(findings, "agent: `builtin` and `command` exclude each other".into());
      } else if let Err(e) = acpira_agent::profile::Profile::load(dir) {
        err(findings, format!("agent: {e}"));
      }
    } else {
      match agent.get("command").and_then(Value::as_str).filter(|c| !c.trim().is_empty()) {
        None => err(findings, "agent: `command` (another ACP agent) or `builtin` (Acpira's loop with a profile) is needed".into()),
        // An agent CLI the user has not installed yet is no fault of the plugin: its tile offers the install line. A
        // program meant to ship inside the plugin still has to be there
        Some(command) => {
          let program = crate::plugins::mcp::strip_dir(command);
          if program != command || program.contains(['/', '\\']) || program_in(dir, program).is_some() || which(program) {
            command_exists(&[program.to_owned()], dir, which, "agent", findings);
          } else {
            findings.notes.push(format!("agent: `{program}` is not on PATH yet; the agent shows as not installed until it is"));
          }
        }
      }
    }
    if let Some(mark) = agent.get("mark")
      && let Err(e) = crate::acp::agents::registry::mark_paths(mark)
    {
      err(findings, format!("agent: {e}"));
    }
    if agent.get("vendor").is_some_and(|v| !v.is_string()) {
      err(findings, "agent: `vendor` is a string".into());
    }
  }
  if let Some(hooks) = &m.contributes.hooks {
    if !hooks.is_object() {
      err(findings, "hooks: expected the object of .agents/hooks.json".into());
    } else {
      match crate::hooks::parse(dir, &hooks.to_string()) {
        Err(e) => err(findings, format!("hooks: {e}")),
        Ok(c) => {
          if !c.unknown.is_empty() {
            err(findings, format!("hooks: unknown keys {}", c.unknown.join(", ")));
          }
          for p in &c.context {
            if !p.is_file() {
              err(findings, format!("hooks: context file {} not found in the plugin", p.strip_prefix(dir).unwrap_or(p).display()));
            }
          }
        }
      }
    }
  }
  if let Some(rt) = &m.contributes.runtime {
    match (rt.command.is_empty(), rt.main.as_deref()) {
      (false, Some(_)) => err(findings, "runtime: `command` and `main` exclude each other".into()),
      (true, None) => err(findings, "runtime: needs a `command`, or a `main` module whose default export is the plugin's definition".into()),
      (false, None) => command_exists(&rt.command, dir, which, "runtime", findings),
      (true, Some(main)) => {
        source_file(dir, main, "runtime: main", findings);
        if !which("node") {
          err(findings, "runtime: `main` runs with Node 18 or newer, which is not on PATH".into());
        }
      }
    }
    let served = m.contributes.tools.iter().any(|t| t.command.is_empty())
      || !m.contributes.schedules.is_empty()
      || m.contributes.commands.iter().any(|c| c.prompt.is_none());
    if rt.hooks.is_empty() && rt.events.is_empty() && rt.methods.is_empty() && !served {
      err(findings, "runtime: declares no `hooks`, `events`, `methods`, tools, schedules or prompt-less commands, so it would never be called".into());
    }
    for name in rt.methods.iter().filter(|n| !valid_method_name(n)) {
      err(findings, format!("runtime: method `{name}` must be lowercase letters, digits, `_`, `.` and `-`, starting with a letter, at most 48 characters"));
    }
    for h in rt.hooks.iter().filter(|h| !RUNTIME_HOOKS.contains(&h.as_str())) {
      findings.notes.push(format!("runtime: hook `{h}` is not one this Acpira calls"));
    }
    for e in &rt.events {
      // `turn.*` and `turn.ended` both name the `turn` family; the names inside a family are not listed here
      let family = e.strip_suffix(".*").unwrap_or(e).split('.').next().unwrap_or(e);
      if e != "*" && !RUNTIME_EVENTS.iter().any(|k| *k == e || k.split('.').next() == Some(family)) {
        findings.notes.push(format!("runtime: event `{e}` is not one this Acpira sends"));
      }
    }
    match rt.scope.as_deref() {
      None => {}
      Some(SCOPE_AGENT) if m.contributes.agent.is_some() => {}
      Some(SCOPE_AGENT) => err(findings, "runtime: scope `agent` needs the plugin's own `agent`".into()),
      Some(other) => err(findings, format!("runtime: scope `{other}` is not `agent`")),
    }
    if rt.timeout.is_some_and(|s| s > RUNTIME_TIMEOUT_SECS) {
      findings.notes.push(format!("runtime: timeout cut to {RUNTIME_TIMEOUT_SECS} s"));
    }
  }
  let mut jobs = std::collections::HashSet::new();
  for j in &m.contributes.schedules {
    let at = format!("schedule `{}`", j.id);
    if !valid_command_name(&j.id) {
      err(findings, format!("{at}: the id must be lowercase letters, digits, `-` and `_`, starting with a letter, at most {NAME_MAX} characters"));
    }
    if !jobs.insert(j.id.as_str()) {
      err(findings, format!("{at} is declared twice"));
    }
    if let Err(e) = crate::plugins::schedule::Cron::parse(&j.cron) {
      err(findings, format!("{at}: cron {e}"));
    }
    if let Some(c) = j.catch_up.as_deref().filter(|c| !CATCH_UP.contains(c)) {
      err(findings, format!("{at}: catchUp `{c}` is not one of {}", CATCH_UP.join(", ")));
    }
    if j.timeout.is_some_and(|t| t > SCHEDULE_TIMEOUT_MAX_SECS) {
      findings.notes.push(format!("{at}: timeout cut to {SCHEDULE_TIMEOUT_MAX_SECS} s"));
    }
    if m.contributes.runtime.is_none() {
      err(findings, format!("{at}: needs a runtime to run it (`schedules` in `definePlugin`, or the SDK's `schedule`)"));
    }
  }
  check_dependencies(m, findings);
  if let Some(e) = &m.exports {
    source_file(dir, e, "exports", findings);
  }
  check_packages(dir, findings);
  if let Some(mcp) = &m.contributes.mcp {
    for (name, entry) in mcp {
      if name == acpira_shared::host_tools::SERVER {
        err(findings, format!("mcp: the name `{name}` is Acpira's own"));
      }
      if !entry.is_object() {
        err(findings, format!("mcp `{name}`: expected an entry of .mcp.json"));
      } else if let Some(command) = entry.get("command").and_then(Value::as_str) {
        let program = crate::plugins::mcp::strip_dir(command).to_owned();
        command_exists(&[program], dir, which, &format!("mcp `{name}`"), findings);
      } else if entry.get("url").and_then(Value::as_str).is_none() {
        err(findings, format!("mcp `{name}`: needs a `command` or a `url`"));
      }
    }
  }
}

/// `dependencies` / `optionalDependencies`: plugin ids with npm-style ranges. Whether they are installed and in effect is
/// the host's check (`PluginHost::check_dependencies`), against the state
fn check_dependencies(m: &Manifest, findings: &mut Findings) {
  let mut seen = std::collections::HashSet::new();
  for (kind, map) in [("dependencies", &m.dependencies), ("optionalDependencies", &m.optional_dependencies)] {
    for (id, range) in map {
      if !valid_id(id) {
        findings.errors.push(format!("{kind}: `{id}` is not a plugin id; npm packages go in package.json"));
        continue;
      }
      if *id == m.id {
        findings.errors.push(format!("{kind}: a plugin cannot depend on itself"));
      }
      if !seen.insert(id.as_str()) {
        findings.errors.push(format!("`{id}` is in both dependencies and optionalDependencies"));
      }
      if let Err(e) = version_req(range) {
        findings.errors.push(format!("{kind} `{id}`: {e}"));
      }
    }
  }
}

/// `version` satisfies the npm-style `range`. `version` is free text in the manifest: `1` and `1.2` count as `1.0.0`
/// and `1.2.0`, a leading `v` is dropped, anything else satisfies nothing
pub fn version_meets(version: &str, range: &str) -> bool {
  let Some(v) = coerce_version(version) else { return false };
  version_req(range).is_ok_and(|r| r.matches(&v))
}

fn coerce_version(text: &str) -> Option<semver::Version> {
  let t = text.trim().trim_start_matches(['v', 'V']);
  if let Ok(v) = semver::Version::parse(t) {
    return Some(v);
  }
  let parts: Vec<u64> = t.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
  match parts.as_slice() {
    [major] => Some(semver::Version::new(*major, 0, 0)),
    [major, minor] => Some(semver::Version::new(*major, *minor, 0)),
    _ => None,
  }
}

/// An npm range: `^1.2`, `~0.3.1`, `>=1.2 <2` (comparators by space or comma), `1.x`, `*`. A bare version, partial
/// or not, means that version as npm reads it: `1.2.3` exactly, `1.2` any `1.2.x`, `1` any `1.x` (the semver crate
/// alone would read a bare `1.2` as `^1.2`). `||` alternatives and hyphen ranges are refused
pub fn version_req(range: &str) -> Result<semver::VersionReq, String> {
  let r = range.trim();
  if r.is_empty() || r == "latest" || r == "*" || r == "x" {
    return Ok(semver::VersionReq::STAR);
  }
  if r.contains("||") {
    return Err(format!("`{r}`: alternatives (`||`) are not supported; use one range"));
  }
  if r.contains(" - ") {
    return Err(format!("`{r}`: hyphen ranges are not supported; write `>=a <=b`"));
  }
  let bad = || format!("`{r}` is not a version range (`^1.2`, `~0.3`, `>=1 <2`, `1.x`, `*`)");
  // Comparators, an operator written apart from its version (`>= 1.2`) joined back to it
  let mut comparators: Vec<String> = vec![];
  let mut pending = String::new();
  for token in r.split([' ', ',']).filter(|t| !t.is_empty()) {
    if token.chars().all(|c| "<>=^~".contains(c)) {
      pending.push_str(token);
      continue;
    }
    let token = token.trim_start_matches(['v', 'V']).replace(".x", ".*").replace(".X", ".*");
    let op_len = token.find(|c: char| !"<>=^~".contains(c)).unwrap_or(token.len());
    let (op, ver) = token.split_at(op_len);
    let op = format!("{pending}{op}");
    pending.clear();
    let ver = ver.trim_start_matches(['v', 'V']);
    if ver.is_empty() {
      return Err(bad());
    }
    comparators.push(if op.is_empty() { format!("={ver}") } else { format!("{op}{ver}") });
  }
  if !pending.is_empty() || comparators.is_empty() {
    return Err(bad());
  }
  semver::VersionReq::parse(&comparators.join(", ")).map_err(|_| bad())
}

/// Lockfiles that pin what `node_modules` holds
const LOCKFILES: &[&str] = &["package-lock.json", "npm-shrinkwrap.json", "pnpm-lock.yaml", "yarn.lock", "bun.lock", "bun.lockb"];

/// npm packages: `package.json` lists them, the author installs them (Acpira never runs an install or its scripts), and
/// the lockfile pins them. Native code in them is named, since it runs with the user's rights like the rest
fn check_packages(dir: &Path, findings: &mut Findings) {
  let Ok(bytes) = std::fs::read(dir.join("package.json")) else { return };
  let pkg: Value = match serde_json::from_slice(&bytes) {
    Ok(v) => v,
    Err(e) => {
      findings.errors.push(format!("package.json: {e}"));
      return;
    }
  };
  let listed = ["dependencies", "optionalDependencies"].iter().any(|k| pkg.get(k).and_then(Value::as_object).is_some_and(|d| !d.is_empty()));
  if !listed {
    return;
  }
  if !dir.join("node_modules").is_dir() {
    findings.errors.push("package.json lists dependencies but node_modules is missing: run `npm install --ignore-scripts` in the plugin folder".into());
  }
  if !LOCKFILES.iter().any(|f| dir.join(f).is_file()) {
    findings.errors.push("package.json lists dependencies without a lockfile: install with npm, pnpm, yarn or bun so the versions are pinned".into());
  }
  let mut native = vec![];
  let _ = walk_files(&dir.join("node_modules"), &mut |p| {
    if p.extension().is_some_and(|e| e == "node") && native.len() < 5 {
      native.push(p.strip_prefix(dir).unwrap_or(p).to_string_lossy().replace('\\', "/"));
    }
  });
  if !native.is_empty() {
    findings.notes.push(format!("native code in node_modules ({}): it runs in the plugin's process with your rights", native.join(", ")));
  }
}

fn walk_files(dir: &Path, f: &mut dyn FnMut(&Path)) -> std::io::Result<()> {
  for e in std::fs::read_dir(dir)?.flatten() {
    let t = e.file_type()?;
    if t.is_dir() {
      walk_files(&e.path(), f)?;
    } else if t.is_file() {
      f(&e.path());
    }
  }
  Ok(())
}

/// A module the engine compiles (`runtime.main`, `ui[].entry`, `exports`)
fn source_file(dir: &Path, rel: &str, at: &str, findings: &mut Findings) {
  if !inside(dir, rel).is_some_and(|p| p.is_file()) {
    findings.errors.push(format!("{at} `{rel}` not found in the plugin"));
  } else if !crate::plugins::build::compiled_source(rel) {
    findings.errors.push(format!("{at} `{rel}` must be a .ts, .tsx, .js, .jsx or .mjs module"));
  }
}

/// A command's program: a file of the plugin when it names one (`convert.py`, `bin/tool`), else a program on PATH
fn command_exists(command: &[String], dir: &Path, which: &dyn Fn(&str) -> bool, at: &str, findings: &mut Findings) {
  let Some(program) = command.first().filter(|p| !p.trim().is_empty()) else {
    findings.errors.push(format!("{at}: `command` is empty"));
    return;
  };
  if program_in(dir, program).is_some() || (!program.contains(['/', '\\']) && which(program)) {
    return;
  }
  findings.errors.push(format!("{at}: `{program}` is neither a file of the plugin nor on PATH"));
}

/// `program` as a file inside the plugin folder, when it names one
pub fn program_in(dir: &Path, program: &str) -> Option<std::path::PathBuf> {
  inside(dir, program).filter(|p| p.is_file())
}

/// A stylesheet of the plugin, within `CSS_MAX_BYTES`
fn css_file(dir: &Path, rel: &str, what: &str, findings: &mut Findings) {
  match inside(dir, rel).and_then(|p| p.metadata().ok()).filter(|m| m.is_file()) {
    None => findings.errors.push(format!("{what} `{rel}` not found in the plugin")),
    Some(m) if m.len() > CSS_MAX_BYTES => findings.errors.push(format!("{what} `{rel}` is over {} KB", CSS_MAX_BYTES / 1024)),
    Some(_) => {}
  }
}

/// A relative path that stays inside `dir` (no `..`, not absolute)
pub fn inside(dir: &Path, rel: &str) -> Option<std::path::PathBuf> {
  let p = Path::new(rel);
  if p.is_absolute() || p.components().any(|c| !matches!(c, std::path::Component::Normal(_) | std::path::Component::CurDir)) {
    return None;
  }
  Some(dir.join(p))
}

/// No way out of a declaration: a value cannot end it, open a block or start markup
pub fn safe_css_value(v: &str) -> bool {
  !v.trim().is_empty() && !v.contains([';', '{', '}', '<', '>']) && !v.contains("/*")
}
