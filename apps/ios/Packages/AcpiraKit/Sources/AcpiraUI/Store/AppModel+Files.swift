import AcpiraDesign
import AcpiraModels
import Foundation

// The engine's reads the phone asks for and awaits (panel/api.ts): a session's changed files, a project file with its
// changes, a tool diff's whole files, the session as Markdown. Each request carries a seq the answer echoes; an answer
// that never comes ends as nil after a while. The demo answers from the fixture's own diffs.

/// A file as the file page shows it: its lines with the session's changes put in place, or why it cannot be shown
enum FileContent: Equatable {
  case lines([DiffLine])
  case failed(ProjectError)
}

extension AppModel {
  /// Posts the request `make` builds around a fresh seq and waits for the answer that echoes it
  func ask(_ make: (Int) -> WebviewMsg) async -> HostMsg? {
    guard live else { return nil }
    askSeq += 1
    let seq = askSeq
    return await withCheckedContinuation { done in
      asking[seq] = done
      post(make(seq))
      Task { @MainActor [weak self] in
        try? await Task.sleep(for: .seconds(20))
        self?.answer(seq, nil)
      }
    }
  }

  /// The answer to request `seq` (nil: it timed out)
  func answer(_ seq: Int, _ message: HostMsg?) {
    asking.removeValue(forKey: seq)?.resume(returning: message)
  }

  /// The files the session's edits left changed, in the order it first edited them
  func changes(_ sessionId: String) async -> [ProjectChange] {
    guard live else { return Self.demoChanges(views[sessionId]) }
    if case let .sessionChanges(_, files)? = await ask({ .sessionChanges(sessionId: sessionId, seq: $0) }) { return files }
    return []
  }

  /// A project file as it is now, with what the session changed in it marked
  func file(_ sessionId: String, path: String) async -> FileContent {
    guard live else {
      let lines = Self.demoLines(views[sessionId], path: path)
      return lines.isEmpty ? .failed(.missing) : .lines(lines)
    }
    switch await ask({ .projectFile(sessionId: sessionId, path: path, base: .session, seq: $0) }) {
    case let .projectFile(_, text?, hunks, _)?: return .lines(fileLines(text, hunks: hunks ?? []))
    case let .projectFile(_, _, _, error?)?: return .failed(error)
    default: return .failed(.message(t("mobile.engineNeeded")))
    }
  }

  /// A diff's whole files (CodeBlock.tsx readSource); a subagent's diffs name it
  func diffSource(_ sessionId: String, _ at: DiffLocation, subagentId: String? = nil) async -> FullDiffSource? {
    guard live else { return nil }
    let answer = await ask { .diffSource(sessionId: sessionId, toolCallId: at.toolCallId, nth: at.nth, subagentId: subagentId, seq: $0) }
    if case let .diffSource(_, source)? = answer { return source }
    return nil
  }

  /// The session as Markdown, the engine's own export (the desktop's Copy → Markdown), or as another of its exports
  /// (`format`, the plain text); the demo exports its view
  func sessionMarkdown(_ sessionId: String, format: ExportFormat = .markdown) async -> String? {
    guard live else {
      return views[sessionId].map { view in
        let name = agent(view.agent).name
        return format == .text ? SessionExport.text(view, agentName: name)
          : SessionExport.markdown(view, agentName: name, exportedAt: now)
      }
    }
    // Markdown asks without the field, as before it existed
    let asked = format == .markdown ? nil : format
    if case let .sessionMarkdown(_, markdown?, _)? = await ask({ .sessionMarkdown(id: sessionId, seq: $0, format: asked) }) {
      return markdown
    }
    return nil
  }

  /// Moves a session to another project folder: the agent works there from its next open on
  func move(_ sessionId: String, to cwd: String) {
    if !post(.moveSession(id: sessionId, cwd: cwd)) {
      if let at = summaries.firstIndex(where: { $0.id == sessionId }) { summaries[at].cwd = cwd }
      views[sessionId]?.cwd = cwd
    }
    let title = summaries.first { $0.id == sessionId }?.title ?? views[sessionId]?.title ?? ""
    show(t("session.movedTo", ["title": title, "project": projectName(cwd)]), icon: .folderInput)
  }

  // MARK: - Demo

  /// The fixture's edits by file, as the engine counts them
  static func demoChanges(_ view: SessionView?) -> [ProjectChange] {
    var order: [String] = []
    var counts: [String: (add: Int, del: Int)] = [:]
    for (path, lines) in diffs(view) {
      if counts[path] == nil { order.append(path) }
      let add = lines.filter { $0.kind == .add }.count, del = lines.filter { $0.kind == .del }.count
      counts[path] = ((counts[path]?.add ?? 0) + add, (counts[path]?.del ?? 0) + del)
    }
    return order.map { ProjectChange(path: $0, add: counts[$0]!.add, del: counts[$0]!.del) }
  }

  /// The fixture has no project to read: the file page shows its diffs of that file, one after the other, without
  /// their hunk headers
  static func demoLines(_ view: SessionView?, path: String) -> [DiffLine] {
    diffs(view).filter { $0.path == path }.flatMap(\.lines).filter { $0.kind != .hunk }
  }

  private static func diffs(_ view: SessionView?) -> [(path: String, lines: [DiffLine])] {
    (view?.turns ?? []).flatMap { turn -> [(path: String, lines: [DiffLine])] in
      guard case .agent(let agent) = turn else { return [] }
      return agent.blocks.flatMap { block -> [(path: String, lines: [DiffLine])] in
        guard let call = block.toolCall else { return [] }
        return call.allContents.compactMap { content in
          // A fixture's diff may carry no source; its edit row names the file
          guard case let .diff(lines, source) = content, let path = source?.path ?? call.target else { return nil }
          return (path, lines)
        }
      }
    }
  }
}

/// FileViewer.tsx viewRows: the file as it is now, the removed lines put back where they stood, as diff lines (each
/// text starting with its sign, as the wire's do)
nonisolated func fileLines(_ text: String, hunks: [ProjectHunk]) -> [DiffLine] {
  var lines = text.components(separatedBy: "\n")
  if lines.last == "" { lines.removeLast() }
  lines = lines.map { $0.hasSuffix("\r") ? String($0.dropLast()) : $0 }
  let starts = Dictionary(hunks.map { ($0.at, $0) }, uniquingKeysWith: { a, _ in a })
  var rows: [DiffLine] = []
  var inside = 0
  var old = 0
  func putBack(_ hunk: ProjectHunk) {
    for d in hunk.del ?? [] {
      old += 1
      rows.append(DiffLine(kind: .del, text: "-" + d, oldLine: old, newLine: nil))
    }
  }
  for (i, line) in lines.enumerated() {
    let n = i + 1
    if let hunk = starts[n] {
      putBack(hunk)
      inside = hunk.add
    }
    if inside > 0 {
      inside -= 1
      rows.append(DiffLine(kind: .add, text: "+" + line, oldLine: nil, newLine: n))
    } else {
      old += 1
      rows.append(DiffLine(kind: .ctx, text: " " + line, oldLine: old, newLine: n))
    }
  }
  // Lines removed at the end stood one past the last line
  if let tail = starts[lines.count + 1] { putBack(tail) }
  return rows
}
