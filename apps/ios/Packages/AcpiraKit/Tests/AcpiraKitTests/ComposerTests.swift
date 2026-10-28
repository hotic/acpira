import AcpiraModels
import Foundation
import Testing
@testable import AcpiraUI

// The composer's ported logic, case for case with the TypeScript it mirrors (slashCommands.ts, Mention.tsx,
// personaHits.ts, usageBreakdown.ts, composerControls.ts's sidekick split, ModelPicker's Thinking switch), plus the
// phone's own `@` file search and per-session drafts.

private func decode<T: Decodable>(_ json: String) -> T { try! JSONDecoder().decode(T.self, from: Data(json.utf8)) }

private let commands: [SlashCommand] = decode(#"""
  [{"name":"review","description":"Review the changes","input":{"hint":"<branch>"}},
   {"name":"compact","description":"Summarize the conversation"},
   {"name":"agents:dig","description":"Dig into a bug"},
   {"name":"claude:dig","description":"Dig into a bug"},
   {"name":"init","description":"Create a review file"}]
  """#)

@MainActor struct SlashCommandTests {
  @Test func commandTokenUnderTheCaret() {
    #expect(SlashCommands.commandAt("/rev", caret: 4) == SigilSpan(start: 0, query: "rev"))
    #expect(SlashCommands.commandAt("hi /re", caret: 6) == SigilSpan(start: 3, query: "re"))
    // A path is one token; a slash inside a word or a URL is none
    #expect(SlashCommands.commandAt("/tmp/x", caret: 6) == SigilSpan(start: 0, query: "tmp/x"))
    #expect(SlashCommands.commandAt("a/b", caret: 3) == nil)
    #expect(SlashCommands.commandAt("https://x", caret: 9) == nil)
    // CJK text needs no space before the sigil
    #expect(SlashCommands.commandAt("然后/review", caret: 9) == SigilSpan(start: 2, query: "review"))
    // Only what lies before the caret counts
    #expect(SlashCommands.commandAt("/review now", caret: 11) == nil)
  }

  @Test func mentionTokenUnderTheCaret() {
    #expect(SlashCommands.mentionAt("ask @gp", caret: 7) == SigilSpan(start: 4, query: "gp"))
    #expect(SlashCommands.mentionAt("@", caret: 1) == SigilSpan(start: 0, query: ""))
    #expect(SlashCommands.mentionAt("a@b.com", caret: 7) == nil)
    #expect(SlashCommands.mentionAt("问问@GPT", caret: 6) == SigilSpan(start: 2, query: "GPT"))
    #expect(SlashCommands.mentionAt("@foo bar", caret: 4) == SigilSpan(start: 0, query: "foo"))
    #expect(SlashCommands.mentionAt("@a@b", caret: 4) == nil)
  }

  @Test func completingACommandKeepsItsArguments() {
    #expect(SlashCommands.complete("/re", span: SigilSpan(start: 0, query: "re"), caret: 3, name: "review")
            == PromptEdit(text: "/review ", caret: 8))
    // The rest of the token under the caret goes, the arguments after it stay
    #expect(SlashCommands.complete("/re view x", span: SigilSpan(start: 0, query: "re"), caret: 3, name: "review")
            == PromptEdit(text: "/review view x", caret: 8))
    #expect(SlashCommands.complete("/reXX main", span: SigilSpan(start: 0, query: "re"), caret: 3, name: "review")
            == PromptEdit(text: "/review main", caret: 8))
  }

  @Test func matchingRanksPrefixesFirstAndMergesSkillCopies() {
    #expect(SlashCommands.match(commands, query: "re").map(\.name) == ["review", "init"])
    #expect(SlashCommands.match(commands, query: "RE").map(\.name) == ["review", "init"])
    #expect(SlashCommands.match(commands, query: "dig").map(\.name) == ["agents:dig"])
    #expect(SlashCommands.match(commands, query: "").map(\.name) == ["review", "compact", "agents:dig", "init"])
    #expect(SlashCommands.match(commands, query: "zzz").isEmpty)
  }

  @Test func inputHintWhileTheArgumentsAreEmpty() {
    #expect(SlashCommands.hint(commands, text: "/review ") == "<branch>")
    #expect(SlashCommands.hint(commands, text: "please /review") == "<branch>")
    #expect(SlashCommands.hint(commands, text: "/review main") == nil)
    #expect(SlashCommands.hint(commands, text: "/compact ") == nil)
  }

  @Test func summonsAndDroppedFileTokens() {
    #expect(SlashCommands.summon("ask", name: "GPT", at: 3) == PromptEdit(text: "ask @GPT ", caret: 9))
    #expect(SlashCommands.summon("", name: "GPT", at: 0) == PromptEdit(text: "@GPT ", caret: 5))
    // From the `@` list the typed token is replaced
    #expect(SlashCommands.summon("hi @gp", name: "GPT", at: 3, cut: 3) == PromptEdit(text: "hi @GPT ", caret: 8))
    #expect(SlashCommands.dropToken("see @Comp", span: SigilSpan(start: 4, query: "Comp"), caret: 9)
            == PromptEdit(text: "see ", caret: 4))
  }
}

@MainActor struct MentionPersonaTests {
  @Test func enabledPersonasThenGroups() {
    let settings: SettingsView = decode(ContractSamples.settings)
    let personas = MentionPersona.from(settings, agents: [AgentInfo(id: "codex", name: "Codex")])
    #expect(personas.map(\.id) == ["review", "panel"])
    #expect(personas[0].cli == "Codex" && personas[0].model == "gpt-6" && personas[0].effort == "High" && !personas[0].group)
    #expect(personas[1].group && personas[1].cli == "Review")
    // The demo has no settings, so nothing to summon
    #expect(MentionPersona.from(nil, agents: []).isEmpty)

    var off = settings
    off.subagents[0].enabled = false
    // A group whose members are all off goes with them
    #expect(MentionPersona.from(off, agents: []).isEmpty)
  }

  @Test func hitsLeadTheFileList() {
    let settings: SettingsView = decode(ContractSamples.settings)
    let personas = MentionPersona.from(settings, agents: [])
    let hits = MentionPersona.hits(personas, query: "rev")
    #expect(hits == [FileHit(uri: "acpira-agent:review", path: "Review")])
    #expect(MentionPersona.of(hits[0], in: personas)?.id == "review")
    #expect(MentionPersona.of(FileHit(uri: "file:///w/review.md", path: "review.md"), in: personas) == nil)
    #expect(MentionPersona.hits(personas, query: "").count == 2)
  }
}

@MainActor struct FileSearchTests {
  @Test func onlyTheLatestQueryLands() {
    var posted: [String] = []
    let search = FileSearch { query, _ in posted.append(query); return true }
    search.search("a")
    search.search("ab")
    #expect(posted == ["a", "ab"])
    #expect(search.answered == nil)
    // The slow answer to "a" arrives after "ab" went out
    search.receive(seq: 1, files: [FileHit(uri: "file:///w/a", path: "a")])
    #expect(search.hits.isEmpty && search.answered == nil)
    search.receive(seq: 2, files: [FileHit(uri: "file:///w/ab", path: "ab")])
    #expect(search.hits.map(\.path) == ["ab"] && search.answered == "ab")
  }

  @Test func closingDropsWhatIsInFlight() {
    let search = FileSearch { _, _ in true }
    search.search("x")
    search.close()
    search.receive(seq: 1, files: [FileHit(uri: "file:///w/x", path: "x")])
    #expect(search.hits.isEmpty && search.answered == nil)
  }

  @Test func withoutAnEngineTheDemoFilesAnswer() {
    let search = FileSearch { _, _ in false }
    search.search("comp")
    #expect(search.answered == "comp")
    // Names holding the query before paths holding it
    #expect(search.hits.map(\.path).prefix(2) == ["packages/ui/chat/Composer.tsx", "apps/ios/Packages/AcpiraKit/Sources/AcpiraUI/Chat/Composer.swift"])
    #expect(search.hits.allSatisfy { $0.uri.hasPrefix("file://\(DemoFiles.root)/") })
    search.search("")
    #expect(search.hits.count == DemoFiles.paths.count && search.answered == "")
  }
}

@MainActor struct DraftTests {
  private func model() -> AppModel {
    AppModel(options: LaunchOptions(fixture: "showcase", theme: nil, motion: false, locale: "en", screen: "chat"))
  }

  @Test func aDraftOutlivesLeavingTheChat() {
    let m = model()
    let id = m.fixtureId("showcase")
    m.draft(for: id).text = "half a thought"
    m.parkDraft(id)
    #expect(m.draft(for: id).text == "half a thought")
    #expect(m.draft(for: id) === m.draft(for: id))
    // Sent (emptied), it goes once the chat is left
    m.draft(for: id).text = ""
    m.parkDraft(id)
    #expect(m.drafts[id] == nil)
  }

  @Test func quotesStageTrimmed() {
    let m = model()
    let id = m.fixtureId("showcase")
    m.quote(id, "  the reply's point \n")
    m.quote(id, " \n ")
    #expect(m.draft(for: id).staged == [.quote(text: "the reply's point", comment: nil)])
    m.parkDraft(id)
    #expect(m.drafts[id] != nil)
  }

  @Test func demoDefaults() {
    let m = model()
    #expect(m.compactAt == 300_000)
    #expect(m.personas.isEmpty)
  }
}

@MainActor struct UsageBreakdownTests {
  private let usage: Usage = decode(#"{"used":150000,"size":200000}"#)

  @Test func fillAndTone() {
    #expect(UsageBreakdown.fill(usage) == 0.75)
    #expect(UsageBreakdown.panel(usage, compactAt: 100_000, canCompact: false, running: false)
            == .init(tone: .unsupported, note: t("usage.unsupported")))
    #expect(UsageBreakdown.panel(usage, compactAt: 300_000, canCompact: true, running: false).tone == .limited)
    #expect(UsageBreakdown.panel(usage, compactAt: 100_000, canCompact: true, running: true)
            == .init(tone: .pending, note: t("usage.pendingAfterTurn")))
    #expect(UsageBreakdown.panel(usage, compactAt: 100_000, canCompact: true, running: false).note == t("usage.pendingBeforeSend"))
    #expect(UsageBreakdown.panel(usage, compactAt: 180_000, canCompact: true, running: false) == .init(tone: .ok, note: nil))
    #expect(UsageBreakdown.panel(usage, compactAt: nil, canCompact: true, running: false).tone == .ok)
  }

  @Test func aCompactedSnapshotIsStale() {
    let stale: Usage = decode(#"{"used":150000,"size":200000,"compactedFrom":180000}"#)
    #expect(UsageBreakdown.fill(stale) == UsageBreakdown.staleFill)
    // Its figure predates the compaction, so it never reads as over the threshold
    #expect(UsageBreakdown.panel(stale, compactAt: 100_000, canCompact: true, running: false).tone == .ok)
  }
}

@MainActor struct ComposerChoiceTests {
  private func choices(_ options: [String], set: @escaping (String, String) -> Void = { _, _ in }) -> ModelChoices {
    ModelChoices(controls: SessionControls(options: options.map { decode($0) }), onSetConfig: set)
  }

  @Test func thinkingSwitchWhereTheAgentCanTurnItOff() {
    var sets: [String] = []
    let deepseek = #"{"id":"thinking","name":"Thinking","category":"thought_level","value":"off","options":[{"id":"off","name":"off"},{"id":"low","name":"low"},{"id":"high","name":"high"}]}"#
    let effort = choices([deepseek]) { sets.append("\($0)=\($1)") }.effort
    #expect(effort?.levels.map(\.id) == ["low", "high"])
    #expect(effort?.thinking?.on == false)
    // Back on at High
    effort?.thinking?.set(true)
    #expect(sets == ["thinking=high"])
    // Without an off there is no switch
    let codex = #"{"id":"reasoning_effort","name":"Effort","category":"thought_level","value":"low","options":[{"id":"low","name":"Low"},{"id":"high","name":"High"}]}"#
    #expect(choices([codex]).effort?.thinking == nil)
  }

  @Test func aStandardThinkingPairIsASwitch() {
    var sets: [String] = []
    let model = #"{"id":"model","name":"Model","category":"model","value":"opus-4-6","options":[{"id":"opus-4-6","name":"Claude Opus 4.6"},{"id":"opus-4-6-thinking","name":"Claude Opus 4.6 Thinking"}]}"#
    let effort = choices([model]) { sets.append("\($0)=\($1)") }.effort
    #expect(effort?.levels.isEmpty == true)
    #expect(effort?.thinking?.on == false && effort?.thinking?.disabled == false)
    effort?.thinking?.set(true)
    #expect(sets == ["model=opus-4-6-thinking"])
  }

  @Test func theSidekickHasItsOwnPlace() {
    let sidekick = #"{"id":"sidekick","name":"Sidekick","category":"_sidekick","value":"none","options":[{"id":"none","name":"None"},{"id":"gpt-6","name":"GPT-6"}]}"#
    let level = #"{"id":"sidekick_effort","name":"Sidekick effort","category":"_sidekick_effort","value":"high","options":[{"id":"low","name":"low"},{"id":"high","name":"high"}]}"#
    let split = ComposerControls.split([decode(sidekick), decode(level)])
    #expect(split.sidekick.map(\.id) == ["sidekick"] && split.sidekickEffort.map(\.id) == ["sidekick_effort"])
    #expect(split.other.isEmpty && split.reasoning.isEmpty && split.models.isEmpty)
    let c = choices([sidekick, level])
    #expect(c.sidekick?.value == ComposerControls.noSidekick && c.sidekickEffort?.id == "sidekick_effort")
    // The sidekick's level is never the lead's
    #expect(c.effort == nil)
  }
}
