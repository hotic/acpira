import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import AcpiraSync
import Foundation
import Observation
import SwiftUI

// Projects and sections of the session list: what the folder menus list, moving, placing and removing projects,
// and the user's sections

extension AppModel {
  // MARK: - Projects

  /// Every known project for the folder menus, the latest first: folders added by hand, the ones sessions work in, then
  /// the registered ones no session used yet
  var projects: [String] {
    var seen = Set(removedProjects)
    let used = summaries.sorted { $0.updatedAt > $1.updatedAt }.map(\.cwd)
    return (addedProjects + used + projectOrder).filter { !$0.isEmpty && !isChat($0) && seen.insert($0).inserted }
  }

  /// A chat's folder: directly inside `chatRoot`, in either separator (sessionTree.ts isChatFolder)
  func isChat(_ cwd: String) -> Bool {
    guard let root = chatRoot, !root.isEmpty, let cut = cwd.lastIndex(where: { $0 == "/" || $0 == "\\" }) else { return false }
    return cut > cwd.startIndex && cwd.index(after: cut) < cwd.endIndex && cwd[..<cut] == root
  }

  /// A new session in `cwd` starts as a chat: no folder picked, or one picked from a chat (Shell.tsx `newIn`: from a
  /// chat, "new" is another chat). Only where the engine says where chats live
  func startsChat(_ cwd: String) -> Bool { chatRoot != nil && (cwd.isEmpty || isChat(cwd)) }

  /// Where a session sits, as a row's second line says it: its project's name, or "Chats"
  func placeName(_ cwd: String) -> String { isChat(cwd) ? t("session.group.chats") : projectName(cwd) }

  /// The SSH host a folder lives on, nil for a local folder; `view` is nil until the engine has said how that host is
  func host(of cwd: String) -> (folder: RemoteFolder, view: ConnectionView?)? {
    guard let folder = RemoteFolder.parse(cwd) else { return nil }
    return (folder, connections.first { $0.alias == folder.alias })
  }

  /// The built-in agent (BUILTIN_AGENT_ID)
  static let builtinAgent: AgentId = "acpira"


  /// The by-project view's projects (sessionTree.ts buildSessionTree): the user's order first, then every other folder a
  /// session ran in, the latest first. Folders taken off the list stay off, and so does a host folder nobody added (a
  /// host's list brings every folder it ever ran a session in; sessionTree.ts offList); their sessions still show in a
  /// search
  var listedProjects: [String] {
    let registered = projectOrder + addedProjects
    var seen = Set(removedProjects)
    let used = summaries.sorted { $0.updatedAt > $1.updatedAt }.map(\.cwd)
      .filter { !$0.hasPrefix(RemoteFolder.scheme) || registered.contains($0) }
    // Chats gather in their own group, never as projects
    return (registered + used).filter { !$0.isEmpty && !isChat($0) && seen.insert($0).inserted }
  }

  /// The project leads the by-project view: registered if it was not, then moved before the first one (the desktop
  /// drags it there)
  func moveProjectToTop(_ cwd: String) {
    guard listedProjects.first != cwd else { return }
    if live {
      if !projectOrder.contains(cwd) { post(.categoryOp(op: .addProject(cwd: cwd))) }
      post(.categoryOp(op: .reorderProject(cwd: cwd, before: projectOrder.first { $0 != cwd })))
      return
    }
    projectOrder.removeAll { $0 == cwd }
    projectOrder.insert(cwd, at: 0)
  }

  /// Takes a folder off the list (its sessions stay and still show in a search); the toast puts it back
  func removeProject(_ cwd: String) {
    if !post(.categoryOp(op: .removeProject(cwd: cwd))) {
      projectOrder.removeAll { $0 == cwd }
      if !removedProjects.contains(cwd) { removedProjects.append(cwd) }
    }
    show(t("mobile.project.removed", ["name": projectName(cwd)]), icon: .folder,
         action: ToastAction(label: t("common.undo")) { [weak self] in self?.addProject(cwd) })
  }

  /// Files a folder into a section at its end, or takes it out (`section` nil): the project menu's "Move to section"
  func setProjectSection(_ cwd: String, _ section: String?) {
    if post(.categoryOp(op: .setProjectSection(cwd: cwd, section: section))) { return }
    for i in sections.indices { sections[i].projects.removeAll { $0 == cwd } }
    if let section, let at = sections.firstIndex(where: { $0.id == section }) { sections[at].projects.append(cwd) }
  }

  /// Creates a custom project section, optionally filing `project` into it in the same edit
  func createSection(_ name: String, project: String? = nil) {
    let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
    let label = trimmed.isEmpty ? t("session.section.untitled") : trimmed
    let id = "sec-\(UUID().uuidString.prefix(8).lowercased())"
    if post(.categoryOp(op: .createSection(id: id, name: label, project: project))) { return }
    if let project {
      for i in sections.indices { sections[i].projects.removeAll { $0 == project } }
    }
    sections.append(SessionSection(id: id, name: label, projects: project.map { [$0] } ?? []))
  }

  func renameSection(_ id: String, to name: String) {
    let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return }
    if post(.categoryOp(op: .renameSection(id: id, name: trimmed))) { return }
    if let at = sections.firstIndex(where: { $0.id == id }) { sections[at].name = trimmed }
  }

  /// Deletes a section; its projects remain on the list under "Projects"
  func deleteSection(_ id: String) {
    if post(.categoryOp(op: .deleteSection(id: id))) { return }
    sections.removeAll { $0.id == id }
  }

  func collapseSection(_ id: String, _ collapsed: Bool) {
    if post(.categoryOp(op: .collapseSection(id: id, collapsed: collapsed))) { return }
    if let at = sections.firstIndex(where: { $0.id == id }) { sections[at].collapsed = collapsed ? true : nil }
  }

  func reorderSection(_ id: String, before: String?) {
    if post(.categoryOp(op: .reorderSection(id: id, before: before))) { return }
    guard let from = sections.firstIndex(where: { $0.id == id }) else { return }
    let moving = sections.remove(at: from)
    let at = before.flatMap { anchor in sections.firstIndex { $0.id == anchor } } ?? sections.endIndex
    sections.insert(moving, at: at)
  }

  /// A dragged folder dropped in the list (SessionList.tsx `placeProject`): filed in `section` before `before` (at its
  /// end without one), or out of any section and moved before `before` among the rest (to the end of the order without
  /// one). The engine reorders registered folders only, so a folder listed from its sessions alone is registered first
  func placeProject(_ cwd: String, section: String?, before: String?) {
    if live {
      if let section {
        post(.categoryOp(op: .setProjectSection(cwd: cwd, section: section, before: before)))
        return
      }
      if !projectOrder.contains(cwd) { post(.categoryOp(op: .addProject(cwd: cwd))) }
      if sections.contains(where: { $0.projects.contains(cwd) }) { post(.categoryOp(op: .setProjectSection(cwd: cwd))) }
      post(.categoryOp(op: .reorderProject(cwd: cwd, before: before)))
      return
    }
    for i in sections.indices { sections[i].projects.removeAll { $0 == cwd } }
    if let section, let at = sections.firstIndex(where: { $0.id == section }) {
      let to = before.flatMap { sections[at].projects.firstIndex(of: $0) } ?? sections[at].projects.endIndex
      sections[at].projects.insert(cwd, at: to)
      return
    }
    projectOrder.removeAll { $0 == cwd }
    projectOrder.insert(cwd, at: before.flatMap { projectOrder.firstIndex(of: $0) } ?? projectOrder.endIndex)
  }

  /// The desktop project menu's "Copy path": the folder as the engine names it (`ssh://alias/path` on a host)
  func copyPath(_ cwd: String) {
    UIPasteboardBridge.copy(cwd)
    show(t("session.project.copied"), icon: .folder)
  }

  func toggleOpen(_ cwd: String) {
    if openProjects.remove(cwd) == nil { openProjects.insert(cwd) }
  }

  /// A folder picked by hand: first in the folder menus, and back on the list if it was taken off (the engine registers
  /// it at the end of the user's order)
  func addProject(_ path: String) {
    let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return }
    addedProjects.removeAll { $0 == trimmed }
    addedProjects.insert(trimmed, at: 0)
    if !post(.categoryOp(op: .addProject(cwd: trimmed))) {
      removedProjects.removeAll { $0 == trimmed }
      if !projectOrder.contains(trimmed) { projectOrder.append(trimmed) }
    }
  }
}
