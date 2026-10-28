import AcpiraDesign
import AcpiraMarkdown
import AcpiraModels
import Foundation
import Observation

// What the chat's composer keeps in the app model: each session's unsent draft (useComposerDraft.ts), the `@` file
// search the engine answers (`searchFiles` → `files`), the summonable personas, and the quote and compact actions.

/// One chat's unsent prompt and the attachments staged with it. An object per session, so typing redraws the composer
/// alone, not every view that reads the model
@Observable
final class ComposerDraft {
  var text = ""
  var staged: [Attachment] = []
  /// A sent prompt the composer is rewriting (AppModel+Turns.swift)
  var edit: TurnEdit?
  /// An edit sent and not yet answered: its request id, and what goes back into the composer if the engine refuses it
  @ObservationIgnored var pending: (request: String, edit: TurnEdit, text: String, staged: [Attachment])?

  var isEmpty: Bool { text.isEmpty && staged.isEmpty && edit == nil && pending == nil }
}

/// The `@` list's file search. Each query goes out with a new seq and only the reply to the latest one lands, so a slow
/// answer to an earlier query never replaces the newer list; closing the list invalidates whatever is in flight. Without
/// an engine the demo project's files answer at once
@Observable
final class FileSearch {
  /// The hits and the query they answer (nil until the first reply, so an empty query is not taken as answered)
  private(set) var hits: [FileHit] = []
  private(set) var answered: String?
  @ObservationIgnored private(set) var seq = 0
  @ObservationIgnored private var asked: String?
  /// Posts `searchFiles`; false without an engine
  @ObservationIgnored private let post: (String, Int) -> Bool

  init(post: @escaping (String, Int) -> Bool) { self.post = post }

  func search(_ query: String) {
    seq += 1
    asked = query
    if !post(query, seq) { receive(seq: seq, files: DemoFiles.search(query)) }
  }

  /// The engine's `files` reply; one for an older seq is dropped
  func receive(seq: Int, files: [FileHit]) {
    guard seq == self.seq, let asked else { return }
    hits = files
    answered = asked
  }

  /// The list closed: a late reply cannot refill it
  func close() {
    seq += 1
    asked = nil
    if answered != nil || !hits.isEmpty {
      hits = []
      answered = nil
    }
  }
}

/// The demo's stand-in for the engine's walk of the project folder
enum DemoFiles {
  static let root = "/Users/dev/projects/acpira"
  static let paths = [
    "README.md", "package.json", "tsconfig.json", "src/args.ts", "src/foo.ts", "src/cli/args.ts", "src/cli/index.ts",
    "docs/dev/ios.md", "docs/dev/webview.md", "packages/ui/chat/Composer.tsx", "packages/shared/protocol.ts",
    "apps/ios/Packages/AcpiraKit/Sources/AcpiraUI/Chat/Composer.swift", "rust/crates/acpira-host/src/runtime.rs",
    "assets/screenshot.png",
  ]

  /// Case-insensitive: names holding the query first, then paths holding it, at most 20 (the engine's cap); a bare `@`
  /// lists them all
  static func search(_ query: String) -> [FileHit] {
    let q = query.lowercased()
    let named = paths.filter { q.isEmpty || ($0.split(separator: "/").last.map(String.init) ?? $0).lowercased().contains(q) }
    let rest = paths.filter { !named.contains($0) && $0.lowercased().contains(q) }
    return (named + rest).prefix(20).map { FileHit(uri: "file://\(root)/\($0)", path: $0) }
  }
}

extension AppModel {
  /// The chat's draft, made on first use
  func draft(for id: String) -> ComposerDraft {
    if let kept = drafts[id] { return kept }
    let made = ComposerDraft()
    drafts[id] = made
    return made
  }

  /// A chat left behind keeps what was typed or staged; an empty draft (never used, or sent) goes
  func parkDraft(_ id: String) {
    if drafts[id]?.isEmpty == true { drafts[id] = nil }
  }

  /// Text quoted from the transcript waits in the chat's composer as an annotation (QuoteToolbar's "Add to chat")
  func quote(_ id: String, _ text: String) {
    let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return }
    draft(for: id).staged.append(.quote(text: trimmed, comment: nil))
  }

  /// `-popup quotes`: the last reply's first two blocks staged as quotes (screenshots)
  func seedQuotes(_ id: String) {
    guard draft(for: id).staged.isEmpty, let view = views[id] else { return }
    let reply = view.turns.reversed().lazy.compactMap { turn -> String? in
      guard case .agent(let agent) = turn else { return nil }
      return agent.blocks.lazy.compactMap { if case .text(let text) = $0 { text.markdown } else { nil } }.first
    }.first
    for block in Markdown.parse(reply ?? "").prefix(2) { quote(id, QuoteText.of(block)) }
  }


  /// The personas the composer offers for `@` summons (settings → Subagents)
  var personas: [MentionPersona] { MentionPersona.from(settings, agents: agents) }

  /// The auto-compact threshold the context ring marks (App.tsx compactAt); the demo reads the defaults
  var compactAt: Int? {
    if let settings { return settings.autoCompact ? settings.compactAtTokens : nil }
    return 300_000
  }
}
