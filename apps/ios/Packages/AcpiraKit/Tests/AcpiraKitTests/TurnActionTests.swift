import AcpiraFixtures
import AcpiraModels
import Foundation
import Testing
@testable import AcpiraUI

// The transcript's own actions: which prompts take an edit and what the edit sends (HistoryMessage.tsx), which reply
// offers Retry (Alert.tsx), what the statistics card lists (TurnActions.tsx StatsCard), where goal milestones sit and
// which goal controls apply (Goal.tsx), a background task's Stop (ToolCall.tsx), and what the demo engine plays for each.

private func decode<T: Decodable>(_ json: String) -> T { try! JSONDecoder().decode(T.self, from: Data(json.utf8)) }

private func agentTurns(_ view: SessionView) -> [AgentTurn] {
  view.turns.compactMap { if case .agent(let a) = $0 { a } else { nil } }
}

/// `t()` reads one process-wide table that LabelTests switches between its `init` and its body, so nothing here switches
/// it: labels are compared with `t()` read in the same synchronous body, and figures only in grouping both tables share
@MainActor struct TurnActionTests {
  // MARK: - Statistics

  @Test func outputSpeedLeavesBackgroundToolsOut() {
    let turn = agentTurns(Fixtures.session("goal")!).last!
    // 4120 tokens over the 116.4 s after the first block, less the 1 s foreground edit; the background tasks ran beside
    // the model, so their minutes do not count against it
    #expect(turnOutputTps(turn) == 36)
  }

  @Test func aTallyOfTheLastCallIsTimedFromTheLastTool() {
    // Input + output matches the context the turn left: the tally is the last request's own, timed after the last tool
    let turn: AgentTurn = decode(#"{"startedAt":0,"respondedAt":1000,"endedAt":21000,"usage":{"input":9000,"output":1001,"context":{"used":10000,"size":200000}},"blocks":[{"type":"tool_call","id":"a","kind":"execute","verb":"Run","status":"completed","startedAt":2000,"endedAt":11000}]}"#)
    #expect(turnOutputTps(turn) == 100)
    // A peer that says it counted several calls is timed over the whole stretch, tool time taken out
    var counted = turn
    counted.usage?.modelCalls = 3
    #expect(turnOutputTps(counted) == 91)
    // Too few tokens to time
    var tiny = turn
    tiny.usage?.output = 4
    #expect(turnOutputTps(tiny) == nil)
  }

  @Test func statsListOnlyWhatThePeerReported() {
    let turns = agentTurns(Fixtures.session("goal")!)
    let full = turnStatsContent(turns[1])
    #expect(full.tiles.map(\.label) == [t("stats.duration"), t("stats.firstToken"), t("stats.outputShort")])
    // The cache read rides on the input line as its share, not as a line of its own
    #expect(full.tokens.map(\.label) == [t("stats.input"), t("stats.cachedWrite"), t("stats.output"), t("stats.reasoning")])
    #expect(full.tokens[0].chip != nil)
    #expect(full.context == StatContext(share: "23%", used: "61,200", size: "262,000"))
    // The first reply reported no cache write, reasoning or context: those stay out
    let partial = turnStatsContent(turns[0])
    #expect(partial.tokens.map(\.label) == [t("stats.input"), t("stats.output")])
    #expect(partial.context == nil)
  }

  @Test func theModelIsThePeersOrThePromptsSelection() {
    let controls = Fixtures.session("goal")!.controls
    let settings = TurnSettings(config: ["model": "gpt-5-6-luna"])
    #expect(modelLabel(nil, settings: settings, controls: controls) == "GPT-5.6 Luna")
    let usage: TurnUsage = decode(#"{"model":"gpt-7-preview"}"#)
    #expect(modelLabel(usage, settings: settings, controls: controls) == "gpt-7-preview")
    #expect(modelLabel(nil, settings: nil, controls: controls) == nil)
  }

  // MARK: - Edits and Retry

  @Test func promptsAreEditableOnlyWhileTheSessionIsIdle() {
    var view = Fixtures.session("goal")!
    #expect(canEditPrompts(view))
    view.running = true
    #expect(!canEditPrompts(view))
    view.running = false
    // A queued head while nothing runs reads as sent
    view.queued = [QueuedPrompt(id: "q", text: "next")]
    #expect(!canEditPrompts(view))
    view.queued = nil
    view.status = .error
    #expect(!canEditPrompts(view))
  }

  @Test func keptAttachmentsGoByTheirPlaceInThePrompt() {
    let a = Attachment.file(uri: "file:///a", name: "a"), b = Attachment.file(uri: "file:///b", name: "b")
    #expect(retainedAttachments([a, b, a], staged: [a, b]) == [0, 1])
    // Each staged copy keeps one
    #expect(retainedAttachments([a, b, a], staged: [a, a]) == [0, 2])
    #expect(retainedAttachments([a, b], staged: []) == [])
  }

  @Test func theEditorPicksOnASnapshotOfThePromptsSelections() {
    let live = Fixtures.session("goal")!.controls
    let effort: ConfigControl = decode(#"{"id":"effort","name":"Effort","options":[{"id":"low","name":"Low"},{"id":"high","name":"High"}],"value":"low"}"#)
    let shapes: ModelShapes = ["gpt-5-6-luna": [effort]]
    // The prompt went out on another model: the editor opens on it, with that model's own parameters
    let opened = openTurnControls(live, TurnSettings(config: ["model": "gpt-5-6-luna", "effort": "high"]), shapes: shapes)
    #expect(opened.options.map(\.id) == ["model", "effort"])
    #expect(opened.options.map(\.value) == ["gpt-5-6-luna", "high"])
    // A selection the controls no longer offer yields to the live one
    let gone = openTurnControls(live, TurnSettings(config: ["model": "gpt-4"]), shapes: nil)
    #expect(gone.options.first?.value == "gpt-6-astra-high-thinking")
    // Picking the live model again drops the borrowed parameters; the live controls never move
    let picked = editTurnConfig(opened, live: live, id: "model", value: "gpt-6-astra-high-thinking", shapes: shapes)
    #expect(picked.options.map(\.id) == ["model"])
    #expect(live.options.map(\.id) == ["model"])
  }

  @Test func anEditRequestCarriesTheTrimmedTextAndTheSnapshot() {
    let view = Fixtures.session("goal")!
    guard case .user(let prompt) = view.turns[2] else { Issue.record("no prompt"); return }
    var controls = openTurnControls(view.controls, prompt.settings, shapes: nil)
    controls.modeId = "read-only"
    let edit = TurnEdit(turnIndex: 2, turnCount: view.turns.count, original: prompt, controls: controls)
    let request = editTurnRequest(sessionId: "s", edit: edit, text: "  e2e 也要过\n", staged: [])
    #expect(request.text == "e2e 也要过")
    #expect(request.turnIndex == 2 && request.turnCount == 4 && request.originalText == prompt.text)
    #expect(request.settings == TurnSettings(modeId: "read-only", config: ["model": "gpt-6-astra-high-thinking"]))
  }

  @Test func anEditKeepsTheOriginalsByIndexAndSendsNewOnesAsDrafts() {
    let view = Fixtures.session("goal")!
    guard case .user(var prompt) = view.turns[2] else { Issue.record("no prompt"); return }
    let kept = Attachment.image(blob: "a.png", mimeType: "image/png", name: "a.png")
    let dropped = Attachment.file(uri: "file:///w/b.swift", name: "b.swift")
    prompt.attachments = [kept, dropped]
    let edit = TurnEdit(turnIndex: 2, turnCount: view.turns.count, original: prompt, controls: view.controls)
    let added = Attachment.image(blob: "AAAA", mimeType: "image/jpeg", name: nil)
    let request = editTurnRequest(sessionId: "s", edit: edit, text: "x", staged: [kept, added])
    #expect(request.retainedAttachments == [0])
    #expect(request.attachments == [.image(mimeType: "image/jpeg", data: "AAAA", name: nil)])
  }

  @Test func retryOnlyAnswersARetryableFailureOnTheNewestTurn() {
    #expect(retryableTurn(Fixtures.session("failure")!) == 1)
    // A context that no longer fits goes on by compaction, not a retry
    #expect(retryableTurn(Fixtures.session("context")!) == nil)
    #expect(retryableTurn(Fixtures.session("goal")!) == nil)
    // Declared remedies without retry leave it out
    var view = Fixtures.session("failure")!
    if case .agent(var last) = view.turns[1] {
      last.error?.actions = [.new_session]
      view.turns[1] = .agent(last)
    }
    #expect(retryableTurn(view) == nil)
  }

  // MARK: - Goals

  @Test func goalMilestonesOpenAndCloseTheTurn() {
    let first = splitCodexBlocks(agentTurns(Fixtures.session("goal")!)[0].blocks)
    #expect(first.goalsLead.map(\.event) == [.set])
    #expect(first.goals.map(\.event) == [.paused])
    // One between two actions stays in the process, where it happened
    let blocks: [AgentBlock] = [
      decode(#"{"type":"tool_call","id":"a","kind":"read","verb":"Read","status":"completed"}"#),
      decode(#"{"type":"goal","event":"blocked"}"#),
      decode(#"{"type":"tool_call","id":"b","kind":"read","verb":"Read","status":"completed"}"#),
      decode(#"{"type":"text","markdown":"done"}"#),
    ]
    let mid = splitCodexBlocks(blocks)
    #expect(mid.goalsLead.isEmpty && mid.goals.isEmpty)
    #expect(mid.process.count == 3)
  }

  @Test func setAsGoalHangsOnTheNewestTypedPrompt() {
    var view = Fixtures.session("goal")!
    #expect(lastTypedPrompt(view.turns) == 2)
    // An automatic continuation is skipped; a slash command is no objective
    view.turns.append(.user(decode(#"{"text":"Continue","auto":true,"autoReason":"goal"}"#)))
    #expect(lastTypedPrompt(view.turns) == 2)
    view.turns.append(.user(UserTurn(text: "/compact")))
    #expect(lastTypedPrompt(view.turns) == nil)
    // Only where the agent takes `set` and no goal is set yet
    #expect(!canSetGoal(view))
    view.goal = nil
    #expect(canSetGoal(view))
    view.goalActions = [.pause, .clear]
    #expect(!canSetGoal(view))
  }

  @Test func theHeadOffersTheAdvertisedControlsThatApply() {
    let all: [GoalAction] = [.set, .pause, .resume, .clear]
    var goal = Fixtures.session("goal")!.goal!
    #expect(goalHeadActions(goal, advertised: all) == [.pause, .clear])
    goal.status = .paused
    #expect(goalHeadActions(goal, advertised: all) == [.resume, .clear])
    #expect(goalHeadActions(goal, advertised: nil) == [])
    goal.status = .complete
    #expect(goalHeadActions(goal, advertised: all) == [])
  }

  // MARK: - Background tasks

  @Test func aTaskRowStopsOnlyWhileLiveAndStoppable() {
    let running: AsyncTaskInfo = decode(#"{"id":"t","state":"running","canStop":true}"#)
    #expect(taskRowState(running).stoppable)
    var asked = running
    asked.stopRequested = true
    #expect(taskRowState(asked) == TaskRowState(tag: t("asyncTask.stopping"), tone: .quiet, live: true, stoppable: false))
    var fixed = running
    fixed.canStop = nil
    #expect(!taskRowState(fixed).stoppable)
    var paused = running
    paused.state = "paused"
    #expect(taskRowState(paused).stoppable)
    var failed = running
    failed.state = "failed"
    #expect(taskRowState(failed).tone == .danger && !taskRowState(failed).live)
    var done = running
    done.state = "completed"
    #expect(taskRowState(done).tone == .ok && !taskRowState(done).stoppable)
  }

  // MARK: - Demo engine

  private func model() -> AppModel {
    AppModel(options: LaunchOptions(fixture: "goal", theme: nil, motion: false, locale: "en", screen: "chat"))
  }

  @Test func anEditTakesThePromptAndGivesTheDraftBackOnCancel() {
    let m = model()
    let id = m.fixtureId("goal")
    let draft = m.draft(for: id)
    draft.text = "half a thought"
    m.beginEdit(id, turn: 2)
    #expect(draft.text == "继续，e2e 也要过")
    #expect(draft.edit?.turnIndex == 2)
    // Taking in another prompt keeps the draft from before the first
    m.beginEdit(id, turn: 0)
    m.cancelEdit(id)
    #expect(draft.edit == nil && draft.text == "half a thought")
  }

  @Test func aDemoEditDropsTheLaterTurnsAndReplays() {
    let m = model()
    let id = m.fixtureId("goal")
    m.beginEdit(id, turn: 0)
    m.sendEdit(id, text: "只修单测", attachments: [])
    let view = m.views[id]!
    #expect(view.turns.count == 2 && view.running)
    if case .user(let prompt) = view.turns[0] {
      #expect(prompt.text == "只修单测" && prompt.edited == true)
    } else {
      Issue.record("no prompt")
    }
    #expect(m.draft(for: id).edit == nil)
  }

  @Test func aRefusedEditPutsTheRewriteBack() {
    let m = model()
    let id = m.fixtureId("goal")
    let draft = m.draft(for: id)
    draft.text = "kept"
    m.beginEdit(id, turn: 2)
    let edit = draft.edit!
    // What a send to an engine leaves: an empty composer and the request on its way
    draft.edit = nil
    draft.text = ""
    draft.pending = ("r1", edit, "rewrite", [])
    m.editAnswered("r1", error: "The conversation moved on")
    #expect(draft.edit == edit && draft.text == "rewrite" && draft.pending == nil)
    // An accepted one gives the composer its earlier draft
    draft.edit = nil
    draft.text = ""
    draft.pending = ("r2", edit, "rewrite", [])
    m.editAnswered("r2", error: nil)
    #expect(draft.text == "kept")
  }

  @Test func demoGoalControlsMoveTheGoalAndLeaveAMilestone() {
    let m = model()
    let id = m.fixtureId("goal")
    m.goal(id, .pause)
    #expect(m.views[id]?.goal?.status == .paused)
    guard case .agent(let last)? = m.views[id]?.turns.last, case .goal(let row)? = last.blocks.last else {
      Issue.record("no milestone")
      return
    }
    #expect(row.event == .paused)
    m.goal(id, .clear)
    #expect(m.views[id]?.goal == nil)
    // An empty objective sets nothing
    m.goal(id, .set, objective: "  ")
    #expect(m.views[id]?.goal == nil)
    m.goal(id, .set, objective: "e2e 全绿")
    #expect(m.views[id]?.goal?.objective == "e2e 全绿")
  }

  @Test func demoStopMarksTheTaskStopping() {
    let m = model()
    let id = m.fixtureId("goal")
    m.stopTask(id, task: "task-watch")
    let call = agentTurns(m.views[id]!).last!.blocks.compactMap(\.toolCall).first { $0.asyncTask?.id == "task-watch" }
    #expect(call?.asyncTask?.stopRequested == true)
    #expect(call.map { taskRowState($0.asyncTask!).stoppable } == false)
  }
}
