import AcpiraModels
import Foundation

// slashCommands.ts, Mention.tsx and personaHits.ts: the `/` command and `@` mention tokens under the caret, the command
// list's ranking, completion and input hint, and the summonable personas offered ahead of the files. Offsets count
// Characters, as the composer reads its caret from the field's selection.

/// A `/` or `@` token under the caret: where its sigil sits and what has been typed after it, up to the caret
struct SigilSpan: Equatable {
  var start: Int
  var query: String
}

/// The prompt after a pick, and where the caret goes
struct PromptEdit: Equatable {
  var text: String
  var caret: Int
}

enum SlashCommands {
  /// What may sit right before a sigil for it to count (SIGIL_LEAD): whitespace, or CJK text and its full-width
  /// punctuation, which those languages write without spaces (`问问@GPT`, `然后/review`). Latin letters, digits and ASCII
  /// punctuation keep `a@b.com`, `a/b` and `https://` plain
  private static let cjk: [ClosedRange<UInt32>] = [
    0x1100...0x11ff, 0x2e80...0x2fdf, 0x3000...0x30ff, 0x3130...0x318f, 0x31f0...0x31ff, 0x3400...0x4dbf, 0x4e00...0x9fff,
    0xac00...0xd7af, 0xf900...0xfaff, 0xff00...0xffef, 0x20000...0x323af,
  ]

  static func leads(_ c: Character) -> Bool {
    if c.isWhitespace { return true }
    guard let v = c.unicodeScalars.first?.value else { return false }
    return cjk.contains { $0.contains(v) }
  }

  /// The trailing run of the text before the caret that holds no whitespace, and where it starts
  private static func tail(_ text: String, _ caret: Int) -> (head: [Character], from: Int) {
    let head = Array(text.prefix(max(0, caret)))
    var from = head.count
    while from > 0, !head[from - 1].isWhitespace { from -= 1 }
    return (head, from)
  }

  /// commandAt: the first `/` in the run before the caret that sits at the start, after whitespace or after CJK text;
  /// the query runs from it to the caret (`/tmp/x` is one token, `a/b` none)
  static func commandAt(_ text: String, caret: Int) -> SigilSpan? {
    let (head, from) = tail(text, caret)
    for i in from..<head.count where head[i] == "/" && (i == 0 || leads(head[i - 1])) {
      return SigilSpan(start: i, query: String(head[(i + 1)...]))
    }
    return nil
  }

  /// mentionAt: the last `@` in the run before the caret, when it leads; the query holds no further `@`
  static func mentionAt(_ text: String, caret: Int) -> SigilSpan? {
    let (head, from) = tail(text, caret)
    guard let i = head[from...].lastIndex(of: "@"), i == 0 || leads(head[i - 1]) else { return nil }
    return SigilSpan(start: i, query: String(head[(i + 1)...]))
  }

  /// completeCommand: the token becomes `/name ` (what follows the caret inside it goes too), the text after it stays as
  /// the arguments
  static func complete(_ text: String, span: SigilSpan, caret: Int, name: String) -> PromptEdit {
    let chars = Array(text)
    let head = String(chars[..<min(span.start, chars.count)]) + "/\(name) "
    let rest = String(chars[min(caret, chars.count)...]).drop { !$0.isWhitespace }.drop(while: \.isWhitespace)
    return PromptEdit(text: head + rest, caret: head.count)
  }

  /// matchCommands: name prefixes first, then names containing the query or descriptions with a word starting with it,
  /// each tier in the agent's order; case-insensitive. An empty query lists everything
  static func match(_ commands: [SlashCommand], query: String) -> [SlashCommand] {
    let q = query.lowercased()
    guard !q.isEmpty else { return mergeSkillCopies(commands) }
    func wordStart(_ s: String) -> Bool {
      s.lowercased().split { !($0.isLetter || $0.isNumber) }.contains { $0.hasPrefix(q) }
    }
    let prefix = commands.filter { $0.name.lowercased().hasPrefix(q) }
    let rest = commands.filter { c in
      !c.name.lowercased().hasPrefix(q) && (c.name.lowercased().contains(q) || wordStart(c.description))
    }
    return mergeSkillCopies(prefix + rest)
  }

  /// Devin lists a skill found in both `~/.agents/skills` and `~/.claude/skills` twice (`agents:dig`, `claude:dig`): the
  /// first copy in ranking order stays, and the picked name is sent verbatim
  private static func mergeSkillCopies(_ commands: [SlashCommand]) -> [SlashCommand] {
    var seen = Set<String>()
    return commands.filter { c in
      guard let scope = c.name.firstMatch(of: /^(?:agents|claude):/) else { return true }
      return seen.insert(String(c.name[scope.range.upperBound...])).inserted
    }
  }

  /// commandHint: the input hint of the command the prompt's last token names, while its arguments are still empty
  static func hint(_ commands: [SlashCommand], text: String) -> String? {
    guard let m = text.firstMatch(of: /(?:^|\s)\/(\S+)\s*$/) else { return nil }
    let name = String(m.1)
    return commands.first { $0.name == name }?.input?.hint
  }

  /// A summon (`@name `) written at `at`, replacing `cut` characters there; a space goes before it unless the text
  /// before already ends in one
  static func summon(_ text: String, name: String, at: Int, cut: Int = 0) -> PromptEdit {
    let chars = Array(text)
    let before = String(chars[..<min(at, chars.count)])
    let lead = before.isEmpty || before.last?.isWhitespace == true ? "" : " "
    let token = "\(lead)@\(name) "
    let after = String(chars[min(at + cut, chars.count)...])
    return PromptEdit(text: before + token + after, caret: before.count + token.count)
  }

  /// A file picked for the `@` token: the token leaves the text (the file rides along as an attachment)
  static func dropToken(_ text: String, span: SigilSpan, caret: Int) -> PromptEdit {
    let chars = Array(text)
    let start = min(span.start, chars.count)
    return PromptEdit(text: String(chars[..<start]) + String(chars[min(caret, chars.count)...]), caret: start)
  }
}

// MARK: - Personas

/// A cross-harness subagent (or a group of them) as the `@` list and the summon bean offer it (personaHits.ts
/// MentionPersona): picking one writes `@name `, which the agent reads as "summon this one"
struct MentionPersona: Hashable, Identifiable {
  var id: String
  var name: String
  /// The CLI it runs in (empty for a group)
  var agent: AgentId
  /// The CLI's name, or a group's members joined
  var cli: String?
  var model: String?
  var effort: String?
  var group = false

  /// A persona's hit travels as a FileHit with this scheme
  static let scheme = "acpira-agent:"

  /// App.tsx personas: the enabled personas, then the enabled groups with an enabled member
  static func from(_ settings: SettingsView?, agents: [AgentInfo]) -> [MentionPersona] {
    guard let settings else { return [] }
    let on = settings.subagents.filter(\.enabled)
    let groups = settings.subagentGroups.compactMap { g -> MentionPersona? in
      let names = g.members.compactMap { m in on.first { $0.id == m }?.name }
      guard g.enabled, !names.isEmpty else { return nil }
      return MentionPersona(id: g.id, name: g.name, agent: "", cli: names.joined(separator: t("common.listSep")), group: true)
    }
    return on.map { p in
      MentionPersona(id: p.id, name: p.name, agent: p.agent, cli: agents.first { $0.id == p.agent }?.name ?? p.agent,
                     model: p.model, effort: p.effort.map { ComposerControls.effortLabel(SessionOption(id: $0, name: $0)) })
    } + groups
  }

  /// personaHits: the personas whose name or id holds the query, leading the file list (a bare `@` lists them all:
  /// Swift's `contains("")` is false where JS's `includes('')` is true)
  static func hits(_ personas: [MentionPersona], query: String) -> [FileHit] {
    let q = query.lowercased()
    return personas.filter { q.isEmpty || $0.name.lowercased().contains(q) || $0.id.contains(q) }
      .map { FileHit(uri: scheme + $0.id, path: $0.name) }
  }

  /// personaOfHit
  static func of(_ hit: FileHit, in personas: [MentionPersona]) -> MentionPersona? {
    guard hit.uri.hasPrefix(scheme) else { return nil }
    let id = String(hit.uri.dropFirst(scheme.count))
    return personas.first { $0.id == id }
  }
}
