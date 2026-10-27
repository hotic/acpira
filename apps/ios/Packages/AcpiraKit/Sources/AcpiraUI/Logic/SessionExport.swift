import AcpiraModels
import Foundation

// shared/exportTranscript.ts on the phone: the session as Markdown for reading, or its view as JSON for tooling.
// The Markdown follows the webview's layout (title, agent / project / time, then one section per turn) with the
// export.label.* strings.

enum SessionExport {
  static func markdown(_ view: SessionView, agentName: String, exportedAt: Date) -> String {
    var parts = [
      "# \(view.title)",
      "- **\(t("export.label.agent"))**: \(agentName)\n- **\(t("export.label.project"))**: \(view.cwd)\n- **\(t("export.label.exported"))**: \(exportedAt.iso)",
      "---",
    ]
    for turn in view.turns {
      switch turn {
      case .user(let u):
        if u.auto == true { parts.append("_\(t("export.label.autoCompact"))_"); continue }
        parts.append("### \(t("export.label.user"))\n\n\(u.text)")
      case .agent(let a):
        var out = ["### \(agentName)"]
        for block in a.blocks {
          if let line = self.block(block) { out.append(""); out.append(line) }
        }
        if a.stop == .error, let error = a.error { out.append(""); out.append("> \(t("export.label.error")): \(error.message)") }
        else if let stop = a.stop, stop != .end_turn { out.append(""); out.append("> \(stop.rawValue)") }
        parts.append(out.joined(separator: "\n"))
      }
    }
    return parts.joined(separator: "\n\n") + "\n"
  }

  private static func block(_ b: AgentBlock) -> String? {
    switch b {
    case .thought(let th): "<details><summary>\(t("export.label.thinking"))</summary>\n\n\(th.text)\n\n</details>"
    case .text(let text): text.markdown
    case .plan(let plan):
      plan.entries.map { $0.status == .completed ? "- [x] \($0.title)" : "- [ ] \($0.title)" }.joined(separator: "\n")
    case .toolCall(let call):
      "- **\(call.verb)**" + (call.target.map { " `\($0)`" } ?? "") + (call.meta.map { " (\($0))" } ?? "")
    case .compaction: "_\(t("export.label.compacted"))_"
    case .steer(let s): "### \(t("export.label.user"))\n\n\(s.text)"
    default: nil
    }
  }

  static func json(_ view: SessionView) -> String {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
    return (try? encoder.encode(view)).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
  }

  /// The title as a file name: path separators and control characters out, at most 60 characters
  static func fileName(_ title: String) -> String {
    let cleaned = title.unicodeScalars.map { CharacterSet(charactersIn: "/\\:*?\"<>|").union(.controlCharacters).contains($0) ? "-" : String($0) }.joined()
    let trimmed = cleaned.trimmingCharacters(in: .whitespaces)
    return trimmed.isEmpty ? "session" : String(trimmed.prefix(60))
  }
}
