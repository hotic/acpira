import AcpiraDesign
import AcpiraModels
import Foundation

// Links the app opens itself: a session link (`acpira://session/<id>` and the shells' forms, SessionLinks.swift) in a
// reply, from a notification or from another app opens that session over the current screen, as the desktop's link
// handler does. A file link the prose made of a path (FileLinks.swift) opens the file page of the chat it was tapped in.
// Everything else goes to the system.

extension AppModel {
  /// True when the link was the app's to open
  func openLink(_ url: URL) -> Bool {
    if let file = FileLinks.decode(url) {
      // The innermost chat on the path owns the reply the link was in
      guard let session = path.reversed().lazy.compactMap({ route -> String? in
        if case .chat(let id) = route { return id }
        return nil
      }).first else { return true }
      path.append(.file(session: session, path: file.path, line: file.line))
      return true
    }
    guard let id = SessionLinks.id(ofLink: url.absoluteString) else { return false }
    if path.last == .chat(id) { return true }
    // The engine knows sessions this list may not show yet; the demo only its own
    guard live || views[id] != nil else {
      show(t("mobile.link.missing"), icon: .circleAlert)
      return true
    }
    path.append(.chat(id))
    return true
  }
}

/// What the session menu's Copy puts on the pasteboard (the desktop's session.copy.*)
enum SessionCopy { case link, markdown, cwd }

extension AppModel {
  /// Copies the session's link, its Markdown or its folder; the Markdown is the engine's export, so it is awaited
  func copy(_ id: String, _ kind: SessionCopy) async {
    let text: String?
    switch kind {
    case .link: text = SessionLinks.link(id)
    case .markdown: text = await sessionMarkdown(id)
    case .cwd: text = views[id]?.cwd ?? summaries.first { $0.id == id }?.cwd
    }
    guard let text, !text.isEmpty else {
      show(t("session.copyFailed", ["detail": t("mobile.engineNeeded")]), icon: .triangleAlert)
      return
    }
    UIPasteboardBridge.copy(text)
    switch kind {
    case .link: show(t("session.copied.link"), icon: .link)
    case .markdown: show(t("session.copied.markdown"), icon: .copy)
    case .cwd: show(t("session.copied.cwd"), icon: .folder)
    }
  }
}
