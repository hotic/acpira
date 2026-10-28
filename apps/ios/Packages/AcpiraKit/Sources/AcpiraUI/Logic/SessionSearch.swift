import AcpiraModels
import Foundation

// The list's history search (packages/ui chat/sessionSearch.ts): a session matches when it carries every term, in its
// title, agent or project here, or in its conversation, which the engine searches (`searchSessions`, session_search.rs)
// and answers with the text around the first match. The demo searches its own views the same way.

/// Content hits for one query: session id → the text around its first match
struct SessionHits: Equatable {
  var query: String
  var snippets: [String: String]
}

/// The distinct lower-cased terms of a query
func searchTerms(_ query: String) -> [String] {
  var seen = Set<String>()
  return query.lowercased().split(whereSeparator: \.isWhitespace).map(String.init).filter { seen.insert($0).inserted }
}

enum SessionSearch {
  /// Every term occurs in one of the row's own words: the title, the agent's name or the project's
  static func matches(_ fields: [String], terms: [String]) -> Bool {
    let hay = fields.joined(separator: "\n").lowercased()
    return terms.allSatisfy { hay.contains($0) }
  }

  /// `text` split around every occurrence of the terms, longest first and case-insensitively; matches sit at the odd
  /// indices (markParts)
  static func parts(_ text: String, terms: [String]) -> [String] {
    let sorted = terms.filter { !$0.isEmpty }.sorted { $0.count > $1.count }
    guard !sorted.isEmpty else { return [text] }
    var out: [String] = []
    var plain = ""
    var rest = text[...]
    outer: while !rest.isEmpty {
      for term in sorted {
        if let r = rest.range(of: term, options: [.caseInsensitive, .anchored]) {
          out.append(plain)
          out.append(String(rest[r]))
          plain = ""
          rest = rest[r.upperBound...]
          continue outer
        }
      }
      plain.append(rest.removeFirst())
    }
    out.append(plain)
    return out
  }

  /// The demo's conversation search: every term somewhere in the session's prompts and replies, the snippet from the
  /// first message holding any of them; nil when the title alone matches or a term is missing
  static func snippet(_ view: SessionView, terms: [String]) -> String? {
    var messages: [String] = []
    for turn in view.turns {
      switch turn {
      case .user(let u): messages.append(u.text)
      case .agent(let a):
        for case .text(let b) in a.blocks { messages.append(b.markdown) }
      }
    }
    let all = messages.joined(separator: "\n").lowercased()
    guard terms.allSatisfy({ all.contains($0) }), !matches([view.title], terms: terms) else { return nil }
    for message in messages {
      let flat = message.split(whereSeparator: \.isNewline).joined(separator: " ")
      let hits = terms.compactMap { flat.range(of: $0, options: .caseInsensitive) }
      guard let first = hits.min(by: { $0.lowerBound < $1.lowerBound }) else { continue }
      return excerpt(flat, at: first)
    }
    return nil
  }

  /// A short lead before the match and the rest of the line after it, as the engine cuts it (the row truncates the tail)
  private static func excerpt(_ text: String, at match: Range<String.Index>) -> String {
    let lead = 8
    let start = text.index(match.lowerBound, offsetBy: -lead, limitedBy: text.startIndex) ?? text.startIndex
    let end = text.index(start, offsetBy: 160, limitedBy: text.endIndex) ?? text.endIndex
    return (start > text.startIndex ? "…" : "") + text[start..<max(end, match.upperBound)] + (end < text.endIndex ? "…" : "")
  }
}
