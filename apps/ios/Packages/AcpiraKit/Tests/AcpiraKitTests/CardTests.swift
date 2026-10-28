import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import Foundation
import Testing
@testable import AcpiraUI

// The cards' logic, case for case with the TypeScript it ports: chat/planReview.ts, chat/planSections.ts,
// chat/turnOutcome.ts, shared/turnErrors.ts, chat/dockPlan.ts's plan gate, Notice.tsx's state table, and the demo
// engine's plan, compaction and session-state actions.

private func decode<T: Decodable>(_ json: String) -> T {
  try! JSONDecoder().decode(T.self, from: Data(json.utf8))
}

private func agentTurns(_ name: String) -> [AgentTurn] {
  (Fixtures.session(name)?.turns ?? []).compactMap { if case .agent(let a) = $0 { a } else { nil } }
}

private let claudeOptions: [PermissionOption] = decode("""
  [{"id":"exit-plan-bypass","label":"Yes, and bypass permissions","kind":"allow_always"},
   {"id":"exit-plan-default","label":"Yes, manually approve edits","kind":"allow_once"},
   {"id":"reject","label":"No, keep planning","kind":"reject_once"}]
  """)

@MainActor struct PlanReviewTests {
  @Test func reviseIsTheRejectOnceThatKeepsPlanning() throws {
    let claude: PermissionBlock = decode(#"{"id":"p","title":"Approve Plan","planId":"x","options":[]}"#)
    var review = claude
    review.options = claudeOptions
    #expect(reviseOption(review)?.id == "reject")
    // Kimi's reject-and-exit is another decision: it stays among the extra approvals
    review.options = decode(#"[{"id":"reject_exit","label":"Reject and exit Plan mode","kind":"reject_once"},{"id":"build","label":"Build","kind":"allow_once"}]"#)
    #expect(reviseOption(review) == nil)
    review.options = decode(#"[{"id":"r","label":"拒绝并退出计划模式","kind":"reject_once"}]"#)
    #expect(reviseOption(review) == nil)
  }

  @Test func pendingReviewIsTheNewestTurnsPlanApproval() throws {
    let plan = try #require(Fixtures.session("plan"))
    #expect(pendingPlanReview(plan.turns)?.id == "perm-plan-2")
    #expect(pendingPlanReview(try #require(Fixtures.session("showcase")).turns) == nil)
  }

  @Test func aSavedPlanCutsTheTurn() throws {
    let built = try #require(agentTurns("plan").first)
    let sections = splitPlanSections(built.blocks)
    #expect(sections.map(\.key) == ["start", "plan-1"])
    #expect(sections[0].plan?.id == "plan-1")
    // The continuation after the card: the edit and the reply
    #expect(sections[1].blocks.count == 2)
    // A question's own tool row goes (its record holds it), an empty thought too
    let records = try #require(agentTurns("records").first)
    let kept = splitPlanSections(records.blocks + [.thought(ThoughtBlock(text: "  "))])[0].blocks
    #expect(!kept.contains { $0.toolCall?.id == "t-q" })
    #expect(kept.count == records.blocks.count - 1)
  }

  @Test func headingNamesTheCardAndLeavesTheBody() {
    let doc: PlanDocumentBlock = decode(#"{"id":"d","title":"Ship it","markdown":"\n# Ship it\n\nStep one.","toolCallId":"t","status":"ready"}"#)
    #expect(planHeading(doc).title == "Ship it")
    #expect(planHeading(doc).body == "Step one.")
    var plain = doc
    plain.title = "Plan"
    plain.markdown = "Just steps."
    #expect(planHeading(plain).title == t("plan.title"))
    #expect(planHeading(plain).body == "Just steps.")
  }

  @Test func executorPicksOnlyWhatTheControlOffers() throws {
    let controls = try #require(Fixtures.session("plan")).controls
    let current = PlanExecutor(controls: controls, selected: nil)
    #expect(current.value == "claude-fable-5-1-xhigh")
    #expect(current.pick == ModelPick(configId: "model", value: "claude-fable-5-1-xhigh"))
    #expect(PlanExecutor(controls: controls, selected: "claude-opus-4-8").value == "claude-opus-4-8")
    // A pick the control no longer offers falls back to the model in force
    #expect(PlanExecutor(controls: controls, selected: "gone").value == "claude-fable-5-1-xhigh")
    #expect(PlanExecutor(controls: SessionControls(), selected: nil).pick == nil)
  }

  @Test func planOptionsReadAsPlanChoices() {
    let build: PermissionOption = decode(#"{"id":"b","label":"Build","kind":"allow_once"}"#)
    let revise: PermissionOption = decode(#"{"id":"r","label":"Revise","kind":"reject_once"}"#)
    #expect(permissionOption(build, plan: true).label == t("plan.build"))
    #expect(permissionOption(revise, plan: true).label == t("plan.revise"))
    // Outside a plan the bare words stay the agent's
    #expect(permissionOption(build).label == "Build")
    #expect(permissionOption(claudeOptions[1], plan: true).label == t("permission.plan.manual"))
  }

  @Test func aPlanWaitingForBuildKeepsItsListOutOfTheDock() throws {
    let plan = try #require(Fixtures.session("plan"))
    #expect(dockPlan(plan.turns, running: plan.running) == nil)
    var built = plan
    guard case .agent(var last)? = built.turns.last else { Issue.record("no agent turn"); return }
    last.blocks = last.blocks.map { block in
      if case .planDocument(var d) = block { d.status = "executing"; return .planDocument(d) }
      return block
    }
    built.turns[built.turns.count - 1] = .agent(last)
    #expect(dockPlan(built.turns, running: true)?.count == 3)
  }
}

@MainActor struct TurnOutcomeTests {
  @Test func promptErrorAbsorbsItsErrorNotices() throws {
    let turns = agentTurns("notices")
    // The stream error turn: no failure id, so its error notice joins the closing row
    #expect(absorbedNotices(turns[1]).map(\.id) == ["stream-1"])
    #expect(!displayTurn(turns[1], absorbed: absorbedNotices(turns[1])).blocks.contains { if case .notice = $0 { true } else { false } })
    // An AIR failure names its notice: that one is suppressed instead, nothing absorbed
    let failure = try #require(agentTurns("failure").first)
    #expect(absorbedNotices(failure).isEmpty)
    #expect(failure.error?.failureId == "turn-1:error")
  }

  @Test func turnActionsNeedContentOrAnError() {
    var turn = AgentTurn(blocks: [.notice(decode(#"{"id":"n","severity":"warning","title":"x"}"#))], stop: .end_turn)
    #expect(!showsTurnActions(turn, running: false))
    turn.stop = .error
    #expect(showsTurnActions(turn, running: false))
    #expect(!showsTurnActions(turn, running: true))
  }

  @Test func aRevisedPlanEndsQuietly() {
    let doc: PlanDocumentBlock = decode(#"{"id":"d","title":"T","markdown":"m","toolCallId":"t","status":"rejected"}"#)
    var turn = AgentTurn(blocks: [.planDocument(doc)], stop: .cancelled)
    #expect(revisedPlanEnd(turn))
    #expect(displayTurn(turn, absorbed: []).stop == nil)
    turn.blocks = []
    #expect(!revisedPlanEnd(turn))
  }

  @Test func aLostObservationStopsTheClock() throws {
    let records = try #require(agentTurns("records").first)
    let lost = try #require(records.blocks.first { $0.toolCall?.id == "t-lost" })
    #expect(observedBlock(lost).toolCall?.status == .cancelled)
    #expect(toolVerb(observedBlock(lost).toolCall!) == t("chatgpt.unknownTool"))
    let settled = try #require(records.blocks.first { $0.toolCall?.id == "t-q" })
    #expect(observedBlock(settled) == settled)
  }

  @Test func contextLengthErrors() {
    #expect(isContextLengthError(decode(#"{"message":"x","kind":"context_length_exceeded"}"#)))
    #expect(isContextLengthError(decode(#"{"message":"prompt is too long: 212034 tokens > 200000 maximum"}"#)))
    #expect(isContextLengthError(decode(#"{"message":"This model's maximum context length is 128000 tokens"}"#)))
    #expect(!isContextLengthError(decode(#"{"message":"Upstream model returned 529: overloaded"}"#)))
    #expect(!isContextLengthError(nil))
  }
}

@MainActor struct NoticeStateTests {
  private func agent(_ json: String) -> AgentInfo { decode(json) }

  @Test func readyAndStartingShowNoCard() throws {
    #expect(noticeState(try #require(Fixtures.session("showcase")), agent: nil) == nil)
    #expect(noticeState(try #require(Fixtures.session("connecting")), agent: nil) == nil)
  }

  @Test func signInOffersOnlyTerminalFreeMethods() throws {
    let login = try #require(Fixtures.session("login"))
    let state = try #require(noticeState(login, agent: agent(#"{"id":"grok","name":"Grok Build"}"#)))
    #expect(state.title == t("notice.login.title", ["agent": "Grok Build"]))
    #expect(state.text == t("mobile.notice.loginOnComputer"))
    #expect(state.primary == .login(id: "xai.api_key", name: t("notice.method.grok:xai.api_key")))
    #expect(state.others.isEmpty)
    #expect(state.showRetry)
    #expect(state.detail == login.error)
  }

  @Test func aLockedStoreIsUnlockedOnTheComputer() throws {
    var login = try #require(Fixtures.session("login"))
    login.agent = "devin"
    let state = try #require(noticeState(login, agent: agent(#"{"id":"devin","name":"Devin","accounts":true,"credentialsLocked":true}"#)))
    #expect(state.icon == .keyRound)
    #expect(state.text == t("mobile.notice.unlockOnComputer"))
    #expect(state.primary == nil)
  }

  @Test func aMirrorIsTakenOverOrContinuedAnew() throws {
    let readonly = try #require(Fixtures.session("readonly").flatMap { noticeState($0, agent: nil) })
    #expect(readonly.primary == .takeOver)
    #expect(readonly.others == [.newSession])
    #expect(readonly.text == Fixtures.session("readonly")?.error)
    let closed = try #require(Fixtures.session("closed").flatMap { noticeState($0, agent: nil) })
    #expect(closed.primary == .newSession)
    #expect(closed.others.isEmpty)
  }

  @Test func errorsRetryOrTakeOverTheLock() throws {
    var session = try #require(Fixtures.session("error"))
    let plain = try #require(noticeState(session, agent: nil))
    #expect(plain.primary == .restart)
    #expect(plain.text == session.error)
    #expect(!plain.showRetry)
    session.canTakeOver = true
    let held = try #require(noticeState(session, agent: nil))
    #expect(held.primary == .takeOver)
    #expect(held.title == t("notice.takeOver.title"))
    #expect(held.detail == session.error)
    #expect(held.showRetry)
  }
}

@Suite @MainActor struct CardActionTests {
  private func model(_ fixture: String) -> AppModel {
    AppModel(options: LaunchOptions(fixture: fixture, theme: nil, motion: false, locale: "en", screen: "chat"))
  }

  private func lastAgent(_ m: AppModel, _ id: String) -> AgentTurn? {
    m.views[id]?.turns.reversed().lazy.compactMap { if case .agent(let a) = $0 { a } else { nil } }.first
  }

  private func plan(_ turn: AgentTurn?, _ id: String) -> PlanDocumentBlock? {
    turn?.blocks.lazy.compactMap { if case .planDocument(let d) = $0, d.id == id { d } else { nil } }.first
  }

  @Test func buildAnswersTheCardAndRunsOnTheChosenModel() {
    let m = model("plan")
    let id = m.fixtureId("plan")
    m.buildPlan(id, plan: "plan-2", option: nil, model: ModelPick(configId: "model", value: "claude-opus-4-8"))
    let turn = lastAgent(m, id)
    #expect(plan(turn, "plan-2")?.status == "executing")
    #expect(!(turn?.blocks.contains { if case .permission = $0 { true } else { false } } ?? true))
    #expect(m.views[id]?.controls.options.first?.value == "claude-opus-4-8")
    #expect(m.views[id]?.running == true)
  }

  @Test func aSendWhileThePlanWaitsRevisesIt() {
    let m = model("plan")
    let id = m.fixtureId("plan")
    let before = m.views[id]!.turns.count
    m.send(id, text: "Keep it to one script", attachments: [])
    let view = m.views[id]!
    #expect(view.queued == nil)
    #expect(view.turns.count == before + 2)
    guard case .agent(let revised) = view.turns[before - 1] else { Issue.record("no plan turn"); return }
    #expect(plan(revised, "plan-2")?.status == "rejected")
    #expect(revised.stop == .cancelled)
    #expect(revisedPlanEnd(revised))
    if case .user(let u) = view.turns[before] { #expect(u.text == "Keep it to one script") } else { Issue.record("no prompt") }
  }

  @Test func compactPlaysAnAutomaticTurn() {
    let m = model("context")
    let id = m.fixtureId("context")
    let before = m.views[id]!.turns.count
    m.compact(id)
    let view = m.views[id]!
    #expect(view.turns.count == before + 2)
    if case .user(let u) = view.turns[before] { #expect(u.auto == true && u.autoReason == "compact") } else { Issue.record("no prompt") }
    #expect(view.running)
  }

  @Test func sessionCardsSettleInTheDemo() {
    for fixture in ["login", "readonly", "error"] {
      let m = model(fixture)
      let id = m.fixtureId(fixture)
      switch fixture {
      case "login": m.login(id, method: "xai.api_key")
      case "readonly": m.takeOver(id)
      default: m.restart(id)
      }
      #expect(m.views[id]?.status == .ready)
      #expect(m.views[id]?.error == nil)
    }
  }

  @Test func failureRemedies() {
    let m = model("failure")
    let id = m.fixtureId("failure")
    m.failureAction(id, .new_session)
    #expect(m.path.last == .new(agent: "codex", cwd: m.views[id]?.cwd))
    let before = m.views[id]!.turns.count
    // The failed turn started: retry resends its prompt
    m.failureAction(id, .retry)
    #expect(m.views[id]?.turns.count == before)
    #expect(m.views[id]?.running == true)
  }
}
