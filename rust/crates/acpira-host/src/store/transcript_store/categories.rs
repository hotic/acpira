//! Edits of the session list's layout in `categories.json`: categories, project order and collapse, and sections

use acpira_shared::transcript::{CategoryOp, SessionCategories, SessionCategory, SessionSection};

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
      list.removed_projects.retain(|p| p != cwd);
      let projects = list.projects.get_or_insert_with(Vec::new);
      if !cwd.is_empty() && !projects.contains(cwd) {
        projects.push(cwd.clone());
      }
    }
    // Remembered, so a folder its sessions would still list (one an IDE window ran in, or a registered one) stays gone
    CategoryOp::RemoveProject { cwd } => {
      if let Some(projects) = &mut list.projects {
        projects.retain(|p| p != cwd);
      }
      if !cwd.is_empty() && !list.removed_projects.contains(cwd) {
        list.removed_projects.push(cwd.clone());
      }
      // Off the list is out of its section too
      for s in &mut list.sections {
        s.projects.retain(|p| p != cwd);
      }
    }
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
