import AcpiraFixtures
import AcpiraModels
import Foundation
import Testing
@testable import AcpiraUI

// The local demo engine behind every control: what each action does to the sessions, the list and the stack

@Suite @MainActor struct AppModelTests {
  private func model(_ screen: String = "chat", fixture: String = "showcase") -> AppModel {
    AppModel(options: LaunchOptions(fixture: fixture, theme: nil, motion: false, locale: "en", screen: screen))
  }

  @Test func launchOpensTheFixtureSession() {
    let m = model()
    guard case .chat(let id)? = m.path.last else { Issue.record("no chat on the stack"); return }
    #expect(m.views[id]?.title == Fixtures.session("showcase")?.title)
    #expect(model("list").path.isEmpty)
    // Settings come up as a sheet over the list
    let settings = model("settings")
    #expect(settings.path.isEmpty && settings.settingsOpen)
  }

  @Test func aPromptStartsAStreamingTurn() {
    let m = model()
    let id = m.fixtureId("showcase")
    let before = m.views[id]!.turns.count
    m.send(id, text: "hello", attachments: [])
    let view = m.views[id]!
    #expect(view.running)
    #expect(view.turns.count == before + 2)
    if case .user(let u) = view.turns[before] { #expect(u.text == "hello") } else { Issue.record("no user turn") }
    #expect(m.summaries.first { $0.id == id }?.state == .working)
  }

  @Test func whileRunningPromptsQueueAndStopCancels() {
    let m = model()
    let id = m.fixtureId("showcase")
    m.send(id, text: "first", attachments: [])
    m.send(id, text: "second", attachments: [])
    #expect(m.views[id]?.queued?.map(\.text) == ["second"])
    m.stop(id)
    let view = m.views[id]!
    #expect(!view.running)
    if case .agent(let turn)? = view.turns.last { #expect(turn.stop == .cancelled) } else { Issue.record("no agent turn") }
    // The queue stays for the reader to send, edit or drop
    let item = view.queued!.first!.id
    #expect(m.takeQueued(id, item)?.text == "second")
    #expect(m.views[id]?.queued == nil)
  }

  @Test func newSessionGoesToTheTopOfTheList() {
    let m = model("list")
    let id = m.newSession(agent: "grok", cwd: "/tmp/demo", controls: m.controls(for: "grok"), text: "Fix the build\nmore", attachments: [])
    #expect(m.summaries.first?.id == id)
    #expect(m.summaries.first?.title == "Fix the build")
    #expect(m.projects.first == "/tmp/demo")
    #expect(m.views[id]?.running == true)
  }

  @Test func forkPinRenameDelete() {
    let m = model()
    let id = m.fixtureId("showcase")
    m.fork(id, through: 1)
    guard case .chat(let fork)? = m.path.last, fork != id else { Issue.record("fork not opened"); return }
    #expect(m.views[fork]?.turns.count == 2)
    #expect(m.toast != nil)

    m.togglePin(fork)
    #expect(m.summaries.first { $0.id == fork }?.pinned == true)
    m.rename(fork, to: "  Renamed ")
    #expect(m.views[fork]?.title == "Renamed")
    m.delete(fork)
    #expect(m.views[fork] == nil)
    #expect(!m.path.contains(.chat(fork)))
  }

  @Test func controlsAndCardsUpdateTheSession() {
    let m = model()
    let id = m.fixtureId("showcase")
    let mode = m.views[id]!.controls.modes.last!.id
    m.setMode(id, mode)
    #expect(m.views[id]?.controls.modeId == mode)
    let control = m.views[id]!.controls.options.first!
    let value = control.options.last!.id
    m.setConfig(id, control.id, value)
    #expect(m.views[id]?.controls.options.first?.value == value)

    let q = m.fixtureId("question")
    guard let question = openQuestion(m.views[q]!) else { Issue.record("question fixture has no open question"); return }
    m.answer(q, question: question.id, answers: [:], skipped: true)
    #expect(openQuestion(m.views[q]!) == nil)
  }

  @Test func exportsReadAsTheWebviewsMarkdown() {
    let view = Fixtures.session("showcase")!
    let md = SessionExport.markdown(view, agentName: "Devin", exportedAt: Date(timeIntervalSince1970: 0))
    #expect(md.hasPrefix("# \(view.title)\n"))
    #expect(md.contains("### Devin"))
    #expect(SessionExport.fileName("a/b: c") == "a-b- c")
  }
}
