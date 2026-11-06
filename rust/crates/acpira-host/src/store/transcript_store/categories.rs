//! Edits of the session list's layout in `categories.json`: categories, project order and collapse, and sections

use acpira_shared::transcript::{CategoryOp, ProjectMeta, SessionCategories, SessionCategory, SessionSection};

/// One category edit on the list. Edits naming a category that is gone (deleted in another window) do nothing; a blank
/// name keeps the old one. Reordering only moves a category among its own project's categories
pub fn apply_category_op(list: &mut SessionCategories, op: &CategoryOp) {
  let name_of = |name: &str| Some(crate::util::clip(name.trim(), crate::limits::RENAME_MAX)).filter(|n| !n.is_empty());
  let cats = &mut list.categories;
  match op {
    CategoryOp::Create { id, name, cwd } => {
      if cats.iter().all(|c| &c.id != id) {
        let name = name_of(name).unwrap_or_else(|| crate::i18n::t("session.category.untitled"));
        cats.push(SessionCategory { id: id.clone(), name, cwd: cwd.clone(), collapsed: None });
      }
    }
    CategoryOp::Rename { id, name } => {
      if let (Some(c), Some(name)) = (cats.iter_mut().find(|c| &c.id == id), name_of(name)) {
        c.name = name;
      }
    }
    CategoryOp::Delete { id } => cats.retain(|c| &c.id != id),
    CategoryOp::Collapse { id, collapsed } => {
      if let Some(c) = cats.iter_mut().find(|c| &c.id == id) {
        c.collapsed = collapsed.then_some(true);
      }
    }
    CategoryOp::Reorder { id, before } => {
      let Some(from) = cats.iter().position(|c| &c.id == id) else { return };
      let moving = cats.remove(from);
      let at = before
        .as_ref()
        .and_then(|b| cats.iter().position(|c| &c.id == b && c.cwd == moving.cwd))
        // No anchor: right after the project's last category (its end of the group), else the end of the list
        .or_else(|| cats.iter().rposition(|c| c.cwd == moving.cwd).map(|i| i + 1))
        .unwrap_or(cats.len());
      cats.insert(at, moving);
    }
    CategoryOp::CollapseProject { cwd, collapsed } => {
      list.collapsed_projects.retain(|p| p != cwd);
      if *collapsed {
        list.collapsed_projects.push(cwd.clone());
      }
    }
    CategoryOp::AddProject { cwd } => {
      // A further folder of a project is already listed, as that project
      if list.project_of(cwd) != cwd {
        return;
      }
      list.removed_projects.retain(|p| p != cwd);
      let projects = list.projects.get_or_insert_with(Vec::new);
      if !cwd.is_empty() && !projects.contains(cwd) {
        projects.push(cwd.clone());
      }
    }
    // Remembered, so a folder its sessions would still list (one an IDE window ran in, or a registered one) stays gone.
    // A project's further folders go with it, else their sessions would list them as projects of their own
    CategoryOp::RemoveProject { cwd } => {
      if let Some(projects) = &mut list.projects {
        projects.retain(|p| p != cwd);
      }
      let extra = list.project_meta.remove(cwd).map(|m| m.extra).unwrap_or_default();
      for f in std::iter::once(cwd).chain(extra.iter()) {
        if !f.is_empty() && !list.removed_projects.contains(f) {
          list.removed_projects.push(f.clone());
        }
      }
      // Off the list is out of its section too
      for s in &mut list.sections {
        s.projects.retain(|p| p != cwd);
      }
    }
    CategoryOp::EditProject { cwd, primary, name, extra } => edit_project(list, cwd, primary, name.as_deref().and_then(name_of), extra),
    CategoryOp::ReorderProject { cwd, before } => {
      let Some(projects) = &mut list.projects else { return };
      let Some(from) = projects.iter().position(|p| p == cwd) else { return };
      let moving = projects.remove(from);
      let at = before.as_ref().and_then(|b| projects.iter().position(|p| p == b)).unwrap_or(projects.len());
      projects.insert(at, moving);
    }
    // Two windows seeding at once: whichever lands second finds the list and keeps it
    CategoryOp::SeedProjects { cwds } => {
      if list.projects.is_none() {
        let mut seeded: Vec<String> = vec![];
        for c in cwds.iter().filter(|c| !c.is_empty()) {
          if !seeded.contains(c) {
            seeded.push(c.clone());
          }
        }
        list.projects = Some(seeded);
      }
    }
    CategoryOp::CreateSection { id, name, project } => {
      if list.sections.iter().all(|s| &s.id != id) {
        let name = name_of(name).unwrap_or_else(|| crate::i18n::t("session.section.untitled"));
        list.sections.push(SessionSection { id: id.clone(), name, projects: vec![], collapsed: None });
      }
      if let Some(cwd) = project {
        apply_category_op(list, &CategoryOp::SetProjectSection { cwd: cwd.clone(), section: Some(id.clone()), before: None });
      }
    }
    CategoryOp::RenameSection { id, name } => {
      if let (Some(s), Some(name)) = (list.sections.iter_mut().find(|s| &s.id == id), name_of(name)) {
        s.name = name;
      }
    }
    CategoryOp::DeleteSection { id } => list.sections.retain(|s| &s.id != id),
    CategoryOp::CollapseSection { id, collapsed } => {
      if let Some(s) = list.sections.iter_mut().find(|s| &s.id == id) {
        s.collapsed = collapsed.then_some(true);
      }
    }
    CategoryOp::ReorderSection { id, before } => {
      let Some(from) = list.sections.iter().position(|s| &s.id == id) else { return };
      let moving = list.sections.remove(from);
      let at = before.as_ref().and_then(|b| list.sections.iter().position(|s| &s.id == b)).unwrap_or(list.sections.len());
      list.sections.insert(at, moving);
    }
    // One section per project: filing it anywhere takes it out of the others. A section that is gone files it nowhere
    CategoryOp::SetProjectSection { cwd, section, before } => {
      if cwd.is_empty() {
        return;
      }
      for s in &mut list.sections {
        s.projects.retain(|p| p != cwd);
      }
      if let Some(s) = section.as_ref().and_then(|id| list.sections.iter_mut().find(|s| &s.id == id)) {
        // A drag drops it before a project of that section; anything else (the menu, a gone anchor) appends
        let at = before.as_ref().and_then(|b| s.projects.iter().position(|p| p == b)).unwrap_or(s.projects.len());
        s.projects.insert(at, cwd.clone());
      }
    }
  }
}

/// `CategoryOp::EditProject`. Each folder belongs to one project: a member that was a project of its own is folded in
/// (its categories come along, its own further folders go free), one listed by another project leaves that one. A new
/// primary folder takes the project's place in the list, its section, fold and categories
fn edit_project(list: &mut SessionCategories, cwd: &str, primary: &str, name: Option<String>, extra: &[String]) {
  let primary = if primary.is_empty() { cwd } else { primary };
  if primary.is_empty() {
    return;
  }
  let mut folders: Vec<String> = vec![primary.to_owned()];
  for f in extra {
    if !f.is_empty() && !folders.contains(f) {
      folders.push(f.clone());
    }
  }
  list.project_meta.remove(cwd);
  for f in &folders {
    if f == cwd {
      continue;
    }
    let own = list.project_meta.remove(f).is_some() || list.projects.as_ref().is_some_and(|p| p.contains(f));
    if own {
      drop_project(list, f);
      rename_categories(list, f, primary);
    }
    for m in list.project_meta.values_mut() {
      m.extra.retain(|x| x != f);
    }
  }
  list.project_meta.retain(|_, m| m.name.is_some() || !m.extra.is_empty());
  if primary != cwd {
    let projects = list.projects.get_or_insert_with(Vec::new);
    match projects.iter().position(|p| p == cwd) {
      Some(i) => projects[i] = primary.to_owned(),
      None => projects.push(primary.to_owned()),
    }
    for s in &mut list.sections {
      for p in s.projects.iter_mut().filter(|p| *p == cwd) {
        *p = primary.to_owned();
      }
    }
    for p in list.collapsed_projects.iter_mut().filter(|p| *p == cwd) {
      *p = primary.to_owned();
    }
    rename_categories(list, cwd, primary);
  } else {
    let projects = list.projects.get_or_insert_with(Vec::new);
    if !projects.iter().any(|p| p == primary) {
      projects.push(primary.to_owned());
    }
  }
  list.removed_projects.retain(|p| !folders.contains(p));
  let extra: Vec<String> = folders.split_off(1);
  if name.is_some() || !extra.is_empty() {
    list.project_meta.insert(primary.to_owned(), ProjectMeta { name, extra });
  }
}

/// A project folded into another leaves the list, its sections and the folded projects
fn drop_project(list: &mut SessionCategories, cwd: &str) {
  if let Some(projects) = &mut list.projects {
    projects.retain(|p| p != cwd);
  }
  for s in &mut list.sections {
    s.projects.retain(|p| p != cwd);
  }
  list.collapsed_projects.retain(|p| p != cwd);
}

/// Categories follow their project, since the list files a session by its project, not its own folder
fn rename_categories(list: &mut SessionCategories, from: &str, to: &str) {
  for c in list.categories.iter_mut().filter(|c| c.cwd == from) {
    c.cwd = to.to_owned();
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn edit(list: &mut SessionCategories, cwd: &str, primary: &str, extra: &[&str]) {
    let extra = extra.iter().map(|s| s.to_string()).collect();
    apply_category_op(list, &CategoryOp::EditProject { cwd: cwd.into(), primary: primary.into(), name: None, extra });
  }

  #[test]
  fn a_registered_folder_added_to_a_project_joins_it_with_its_categories() {
    let mut list = SessionCategories { projects: Some(vec!["/a".into(), "/web".into()]), ..Default::default() };
    apply_category_op(&mut list, &CategoryOp::Create { id: "c".into(), name: "x".into(), cwd: "/web".into() });
    edit(&mut list, "/a", "/a", &["/web"]);
    assert_eq!(list.projects, Some(vec!["/a".to_string()]));
    assert_eq!(list.project_meta["/a"].extra, vec!["/web".to_string()]);
    assert_eq!(list.categories[0].cwd, "/a");
    assert_eq!(list.project_of("/web"), "/a");
    assert_eq!(list.other_folders("/web"), vec!["/a".to_string()]);
    assert_eq!(list.other_folders("/a"), vec!["/web".to_string()]);
    // A session started in the further folder does not list it again
    apply_category_op(&mut list, &CategoryOp::AddProject { cwd: "/web".into() });
    assert_eq!(list.projects, Some(vec!["/a".to_string()]));
  }

  #[test]
  fn a_new_primary_takes_the_projects_place_and_the_old_one_stays_a_member() {
    let mut list = SessionCategories { projects: Some(vec!["/z".into(), "/a".into()]), collapsed_projects: vec!["/a".into()], ..Default::default() };
    apply_category_op(&mut list, &CategoryOp::CreateSection { id: "s".into(), name: "w".into(), project: Some("/a".into()) });
    edit(&mut list, "/a", "/b", &["/a"]);
    assert_eq!(list.projects, Some(vec!["/z".to_string(), "/b".to_string()]));
    assert_eq!(list.sections[0].projects, vec!["/b".to_string()]);
    assert_eq!(list.collapsed_projects, vec!["/b".to_string()]);
    assert_eq!(list.project_of("/a"), "/b");
    assert!(!list.project_meta.contains_key("/a"));
  }

  #[test]
  fn a_folder_listed_elsewhere_moves_and_an_emptied_project_keeps_no_entry() {
    let mut list = SessionCategories { projects: Some(vec!["/a".into(), "/b".into()]), ..Default::default() };
    edit(&mut list, "/a", "/a", &["/x"]);
    edit(&mut list, "/b", "/b", &["/x"]);
    assert!(!list.project_meta.contains_key("/a"));
    assert_eq!(list.project_of("/x"), "/b");
    // Removing the project takes its further folder off the list too
    apply_category_op(&mut list, &CategoryOp::RemoveProject { cwd: "/b".into() });
    assert_eq!(list.removed_projects, vec!["/b".to_string(), "/x".to_string()]);
    assert!(list.project_meta.is_empty());
    // Adding it back as a member brings it back on the list
    edit(&mut list, "/a", "/a", &["/x"]);
    assert_eq!(list.removed_projects, vec!["/b".to_string()]);
  }
}
