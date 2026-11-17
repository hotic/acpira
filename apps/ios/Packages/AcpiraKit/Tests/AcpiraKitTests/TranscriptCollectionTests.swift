#if os(iOS)
import AcpiraModels
import SwiftUI
import Testing
import UIKit
@testable import AcpiraUI

@MainActor @Suite(.serialized) struct TranscriptCollectionTests {
  /// A dock may be measured before the collection is mounted. Attaching content must keep that clearance, and later
  /// viewport updates must neither replace the hosted cells nor change their measured heights.
  @Test func dockViewportKeepsHostedContentAndReadingPosition() async throws {
    let control = TranscriptControl()
    let initialSafe = CGRect(x: 0, y: 100, width: 402, height: 574)
    control.updateViewport(initialSafe, animated: false)
    let items = (0..<12).map { index in
      ThreadItem(id: "u\(index)", kind: .user(UserTurn(text: "A measured prompt"), index: index, editable: false, goal: false),
                 exchange: index)
    }
    let host = UIHostingController(rootView: TranscriptCollection(items: items, control: control,
      running: false, motion: false) { _ in })
    host.safeAreaRegions = []
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 402, height: 874))
    window.rootViewController = host
    window.makeKeyAndVisible()
    defer { window.isHidden = true; window.rootViewController = nil }
    host.view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(200))
    let view = try #require(control.view)
    view.layoutIfNeeded()
    #expect(view.safeFrame == initialSafe)
    #expect(abs(view.contentInset.bottom - 200) <= 0.5)
    let cell = try #require(view.visibleCells.first)
    let height = cell.frame.height
    let contentHeight = view.contentSize.height
    view.maintainsBottomAnchor = false
    let offset = view.contentOffset.y

    control.updateViewport(CGRect(x: 0, y: 100, width: 402, height: 474), animated: true)
    #expect(view.dockGlides)
    view.layoutIfNeeded()
    #expect(abs(view.contentInset.bottom - 300) <= 0.5)
    #expect(view.visibleCells.contains { $0 === cell })
    #expect(abs(cell.frame.height - height) <= 0.5)
    #expect(abs(view.contentSize.height - contentHeight) <= 0.5)
    #expect(abs(view.contentOffset.y - offset) <= 0.5)

    control.updateViewport(initialSafe, animated: false)
    #expect(!view.dockGlides)
    view.layoutIfNeeded()
    #expect(abs(view.contentInset.bottom - 200) <= 0.5)
  }

  /// A process closing removes height from the same visible cell. Its former offset is outside the new content,
  /// not a reading anchor to restore: UIScrollView clamps it on its next pass and used to fight the restoration.
  @Test func shrinkingFollowedRowNeverRestoresPastTheNewBottom() throws {
    let layout = UICollectionViewFlowLayout()
    layout.itemSize = CGSize(width: 402, height: 1600)
    let view = TranscriptCollectionView(frame: CGRect(x: 0, y: 0, width: 402, height: 874), collectionViewLayout: layout)
    let source = SingleRow()
    view.register(UICollectionViewCell.self, forCellWithReuseIdentifier: "row")
    view.dataSource = source
    view.contentInset = UIEdgeInsets(top: 100, left: 0, bottom: 200, right: 0)
    let controller = UIViewController()
    controller.view.addSubview(view)
    let window = UIWindow(frame: view.frame)
    window.rootViewController = controller
    window.makeKeyAndVisible()
    defer { window.isHidden = true; window.rootViewController = nil }
    view.reloadData()
    view.layoutIfNeeded()
    view.followBottom(animated: false)
    let anchor = try #require(view.firstVisibleItem())
    let oldOffset = view.contentOffset.y

    layout.itemSize.height = 1000
    layout.invalidateLayout()
    view.layoutIfNeeded()
    view.keepOnScreen(anchor)

    let bottom = max(-view.adjustedContentInset.top,
                     view.contentSize.height - view.bounds.height + view.adjustedContentInset.bottom)
    #expect(oldOffset > bottom + 400)
    #expect(view.contentOffset.y <= bottom + 0.5)
    #expect(view.maintainsBottomAnchor)
  }

  /// A followed reply still gliding toward the end must keep the same visible position when the process above it
  /// folds: waiting for the new bottom to overtake the old offset first moves the reply up, then snaps it back down.
  @Test func shrinkingFollowedRowPreservesItsGapDuringTheGlide() {
    let layout = UICollectionViewFlowLayout()
    layout.itemSize = CGSize(width: 402, height: 1600)
    let view = TranscriptCollectionView(frame: CGRect(x: 0, y: 0, width: 402, height: 874), collectionViewLayout: layout)
    let source = SingleRow()
    view.register(UICollectionViewCell.self, forCellWithReuseIdentifier: "row")
    view.dataSource = source
    let controller = UIViewController()
    controller.view.addSubview(view)
    let window = UIWindow(frame: view.frame)
    window.rootViewController = controller
    window.makeKeyAndVisible()
    defer { view.stopFollowing(); window.isHidden = true; window.rootViewController = nil }
    view.reloadData()
    view.layoutIfNeeded()
    view.followBottom(animated: false)
    view.contentOffset.y -= 40
    view.layoutIfNeeded()
    let gap = view.distanceToEnd

    layout.itemSize.height -= 100
    layout.invalidateLayout()
    view.layoutIfNeeded()

    #expect(gap >= 39.5)
    #expect(abs(view.distanceToEnd - gap) <= 0.5)
  }

  /// Actual hosted process rows, not a geometry formula: a nested LazyVStack has no SwiftUI scroll viewport here
  /// and used to keep re-estimating its height when the UIKit cell moved through the viewport.
  @Test func hostedProcessKeepsMeasuredHeightAcrossViewportMoves() async throws {
    let blocks = (0..<30).map { index in
      AgentBlock.text(TextBlock(markdown: "Process \(index): " + String(repeating: "measured content ", count: index % 4 + 1)))
    }
    let host = UIHostingController(rootView: ProcessHistory(items: groupProcess(blocks))
      .environment(\.hostedTranscript, true).environment(\.motionEnabled, false))
    host.safeAreaRegions = []
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 402, height: 874))
    window.rootViewController = host
    window.makeKeyAndVisible()
    defer { window.isHidden = true; window.rootViewController = nil }
    let proposal = CGSize(width: 402, height: UIView.layoutFittingExpandedSize.height)
    let height = host.sizeThatFits(in: proposal).height
    #expect(height > window.bounds.height)
    for offset in [CGFloat(0), -400, -1200, 0] {
      host.view.frame = CGRect(x: 0, y: offset, width: 402, height: height)
      host.view.layoutIfNeeded()
      try await Task.sleep(for: .milliseconds(40))
      #expect(abs(host.sizeThatFits(in: proposal).height - height) <= 0.5)
    }
  }

  /// The transcript draws under the bars. A UIHostingConfiguration must keep measuring a prompt at its natural
  /// height there, not add the part hidden under the bar to its intrinsic height on every scroll frame.
  @Test func hostedCellHeightDoesNotFollowItsPositionUnderTheBars() async throws {
    let size = NSCollectionLayoutSize(widthDimension: .fractionalWidth(1), heightDimension: .estimated(120))
    let group = NSCollectionLayoutGroup.vertical(layoutSize: size, subitems: [NSCollectionLayoutItem(layoutSize: size)])
    let view = TranscriptCollectionView(frame: CGRect(x: 0, y: 0, width: 402, height: 874),
                                        collectionViewLayout: UICollectionViewCompositionalLayout(section: NSCollectionLayoutSection(group: group)))
    view.contentInsetAdjustmentBehavior = .never
    view.contentInset = UIEdgeInsets(top: 116, left: 0, bottom: 222, right: 0)
    view.motion = false
    let controller = UIViewController()
    controller.additionalSafeAreaInsets = UIEdgeInsets(top: 54, left: 0, bottom: 100, right: 0)
    controller.view.addSubview(view)
    let window = UIWindow(frame: view.frame)
    window.rootViewController = controller
    window.makeKeyAndVisible()
    defer { window.isHidden = true; window.rootViewController = nil }
    let control = TranscriptControl()
    let coordinator = TranscriptCollection.Coordinator(control: control)
    coordinator.connect(view)
    var environment = EnvironmentValues()
    environment.hostedTranscript = true
    environment.motionEnabled = false
    let items = (0..<12).map { index in
      ThreadItem(id: "u\(index)", kind: .user(UserTurn(text: "A measured prompt"), index: index, editable: false, goal: false),
                 exchange: index)
    }
    coordinator.update(items: items, environment: environment, in: view)
    try await Task.sleep(for: .milliseconds(200))
    view.layoutIfNeeded()
    view.maintainsBottomAnchor = false
    view.contentOffset.y = -116
    view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(100))
    let cell = try #require(view.cellForItem(at: IndexPath(item: 0, section: 0)))
    let height = cell.frame.height
    #expect(height > 0)
    #expect(controller.view.safeAreaInsets.top > 0)
    for offset in [CGFloat(-80), -40, 0, -116] {
      view.contentOffset.y = offset
      coordinator.update(items: items, environment: environment, in: view)
      view.layoutIfNeeded()
      try await Task.sleep(for: .milliseconds(100))
      #expect(abs(cell.frame.height - height) <= 0.5)
    }
  }

  @Test(arguments: [320.0, 402.0, 471.0, 768.0])
  func hookNoticeKeepsTheReplyWithinTheHostedRow(width: Double) async throws {
    let control = TranscriptControl()
    let path = "reports/performance/rollback/GameCore-1664000000000-72-before-relic-signature.jar"
    let markdown = "- 已部署但没重启，所以玩家现在跑的还是旧代码，下次重启才生效。旧 jar 备份为 `\(path)`。\n- **提交：** 代码还没提交。\n\n**换层存档卡顿：没修。** 后面的段落也必须保持在对话栏内，不能跟着长路径一起撑宽。"
    let notice = try JSONDecoder().decode(NoticeBlock.self, from: Data(#"""
      {"id":"hooks:layout","severity":"warning",
       "title":"工作区钩子失败：python3 .agents/hooks/check_workspace_after_turn.py --check-changed-files --include-build-output",
       "details":"检查未通过，请先运行项目检查。"}
      """#.utf8))
    let turn = AgentTurn(blocks: [.text(TextBlock(markdown: markdown)), .notice(notice)])
    let items = [ThreadItem(id: "a1", kind: .agent(turn, index: 1, scope: "layout:1", running: false, last: true,
                                                 settings: nil, retry: false, subagents: []), exchange: 0)]
    let host = UIHostingController(rootView: TranscriptCollection(items: items, control: control,
      running: false, motion: false) { _ in })
    host.safeAreaRegions = []
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: width, height: 874))
    window.rootViewController = host
    window.makeKeyAndVisible()
    defer { window.isHidden = true; window.rootViewController = nil }
    host.view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(200))
    let view = try #require(control.view)
    view.layoutIfNeeded()
    #expect(view.contentSize.width <= width + 0.5)
    let measured = UIHostingController(rootView: ThreadItemView(item: items[0]))
      .sizeThatFits(in: CGSize(width: width, height: UIView.layoutFittingExpandedSize.height))
    #expect(measured.width <= width + 0.5)
    let cell = try #require(view.cellForItem(at: IndexPath(item: 0, section: 0)))
    let surfaces = selectionViews(in: cell)
    #expect(!surfaces.isEmpty)
    for surface in surfaces {
      let frame = surface.convert(surface.bounds, to: cell)
      #expect(frame.minX >= 0)
      #expect(frame.maxX <= cell.bounds.width)
    }
  }

  /// A view that starts at the session's tail (`SessionView.earlier`), read at its top: the spinner, the turn the view
  /// was cut in, a prompt. The part ahead lands as a prompt above the cut turn and blocks at that turn's head. Neither
  /// may move what the reader sees: the cut turn grows upwards, its end and the rows after it stay where they were
  @Test func anEarlierPartLandsAboveWhatTheReaderSees() async throws {
    let size = NSCollectionLayoutSize(widthDimension: .fractionalWidth(1), heightDimension: .estimated(120))
    let group = NSCollectionLayoutGroup.vertical(layoutSize: size, subitems: [NSCollectionLayoutItem(layoutSize: size)])
    let view = TranscriptCollectionView(frame: CGRect(x: 0, y: 0, width: 402, height: 874),
                                        collectionViewLayout: UICollectionViewCompositionalLayout(section: NSCollectionLayoutSection(group: group)))
    view.contentInsetAdjustmentBehavior = .never
    view.contentInset = UIEdgeInsets(top: 116, left: 0, bottom: 222, right: 0)
    view.motion = false
    let controller = UIViewController()
    controller.view.addSubview(view)
    let window = UIWindow(frame: view.frame)
    window.rootViewController = controller
    window.makeKeyAndVisible()
    defer { window.isHidden = true; window.rootViewController = nil }
    let control = TranscriptControl()
    var asks = 0, fills = 0
    view.loadEarlier = { asks += 1 }
    view.fillEarlier = { fills += 1 }
    let coordinator = TranscriptCollection.Coordinator(control: control)
    coordinator.connect(view)
    var environment = EnvironmentValues()
    environment.hostedTranscript = true
    environment.motionEnabled = false
    let paragraph = { (name: String) in AgentBlock.text(TextBlock(markdown: "\(name): " + String(repeating: "measured reply text ", count: 30))) }
    func agent(_ index: Int, _ blocks: [String]) -> ThreadItem {
      ThreadItem(id: "a\(index)", kind: .agent(AgentTurn(blocks: blocks.map(paragraph), stop: .end_turn), index: index, scope: "s:\(index)",
                                               running: false, last: false, settings: nil, retry: false, subagents: []), exchange: 0)
    }
    func prompt(_ index: Int) -> ThreadItem {
      ThreadItem(id: "u\(index)", kind: .user(UserTurn(text: "A measured prompt"), index: index, editable: false, goal: false), exchange: index)
    }
    let spinner = ThreadItem(id: ThreadItem.earlierID, kind: .earlier(loading: false), exchange: 0)
    let tail = [prompt(4), agent(5, ["e", "f", "g", "h"]), prompt(6), agent(7, ["i", "j", "k", "l"])]
    coordinator.update(items: [spinner, agent(3, ["c", "d"])] + tail, earlier: ViewEarlier(turns: 3, blocks: 2),
                       environment: environment, in: view)
    try await Task.sleep(for: .milliseconds(200))
    view.layoutIfNeeded()
    // Opened at its end, well below the spinner: nothing is asked for
    #expect(view.maintainsBottomAnchor && asks == 0)

    // The reader jumps to the top (the status bar): the scroll that brings the top within a screen asks
    _ = coordinator.scrollViewShouldScrollToTop(view)
    view.contentOffset.y = -116
    coordinator.scrollViewDidScrollToTop(view)
    view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(100))
    #expect(asks == 1)
    func place(_ id: String) throws -> CGRect {
      let indexPath = try #require(view.readerIndexPath?(id))
      let frame = try #require(view.collectionViewLayout.layoutAttributesForItem(at: indexPath)).frame
      return frame.offsetBy(dx: 0, dy: -view.contentOffset.y)
    }
    let cut = try place("a3"), next = try place("u4")
    #expect(cut.minY < 874 - 222 && cut.height > 100)

    coordinator.update(items: [spinner, prompt(2), agent(3, ["a", "b", "c", "d"])] + tail, earlier: ViewEarlier(turns: 2, blocks: 0),
                       environment: environment, in: view)
    view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(200))
    view.layoutIfNeeded()
    let grown = try place("a3")
    #expect(grown.height > cut.height + 100)
    #expect(abs(grown.maxY - cut.maxY) <= 0.5)
    #expect(abs(try place("u4").minY - next.minY) <= 0.5)
    // The earlier blocks and the prompt went in above the top edge: the reader scrolls up to them
    #expect(try place("u2").maxY < grown.minY + 0.5 && grown.minY < cut.minY - 100)
    // The part landing asked for nothing, and neither does one that changes no row (blocks inside a closed fold),
    // though the top is still within a screen: the next ask is the reader's next scroll
    coordinator.update(items: [spinner, prompt(2), agent(3, ["a", "b", "c", "d"])] + tail, earlier: ViewEarlier(turns: 1, blocks: 7),
                       environment: environment, in: view)
    view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(100))
    view.layoutIfNeeded()
    #expect(asks == 1 && fills == 0)
    // A move that is not the reader's (no finger, no jump) asks for nothing even right under the spinner
    let held = view.contentOffset.y
    view.contentOffset.y = -116 + 60
    #expect(asks == 1)
    _ = coordinator.scrollViewShouldScrollToTop(view)
    view.contentOffset.y = -116 + 50
    coordinator.scrollViewDidScrollToTop(view)
    #expect(asks == 2)
    view.contentOffset.y = held
    view.layoutIfNeeded()

    // The last part: the spinner goes, and still nothing moves
    let before = try place("u4")
    coordinator.update(items: [prompt(0), agent(1, ["x", "y"]), prompt(2), agent(3, ["a", "b", "c", "d"])] + tail, earlier: nil,
                       environment: environment, in: view)
    view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(200))
    view.layoutIfNeeded()
    #expect(view.readerIndexPath?(ThreadItem.earlierID) == nil)
    #expect(abs(try place("u4").minY - before.minY) <= 0.5)
  }

  /// A view of a huge cut turn can be a single short row: nothing to scroll, so the thread asks by itself until its
  /// rows fill the room, and past that (`AppModel.fillEarlier` stops it) the reader's pull at the top asks, once a pull
  @Test func aThreadTooShortToScrollAsksByItselfAndOnAPull() async throws {
    let size = NSCollectionLayoutSize(widthDimension: .fractionalWidth(1), heightDimension: .estimated(120))
    let group = NSCollectionLayoutGroup.vertical(layoutSize: size, subitems: [NSCollectionLayoutItem(layoutSize: size)])
    let view = TranscriptCollectionView(frame: CGRect(x: 0, y: 0, width: 402, height: 874),
                                        collectionViewLayout: UICollectionViewCompositionalLayout(section: NSCollectionLayoutSection(group: group)))
    view.contentInsetAdjustmentBehavior = .never
    view.contentInset = UIEdgeInsets(top: 116, left: 0, bottom: 222, right: 0)
    view.motion = false
    let controller = UIViewController()
    controller.view.addSubview(view)
    let window = UIWindow(frame: view.frame)
    window.rootViewController = controller
    window.makeKeyAndVisible()
    defer { window.isHidden = true; window.rootViewController = nil }
    var asks = 0, fills = 0
    view.loadEarlier = { asks += 1 }
    view.fillEarlier = { fills += 1 }
    let coordinator = TranscriptCollection.Coordinator(control: TranscriptControl())
    coordinator.connect(view)
    var environment = EnvironmentValues()
    environment.hostedTranscript = true
    environment.motionEnabled = false
    func agent(_ index: Int, blocks: Int) -> ThreadItem {
      let blocks = (0..<blocks).map { AgentBlock.text(TextBlock(markdown: "\(index).\($0): " + String(repeating: "measured reply text ", count: 30))) }
      return ThreadItem(id: "a\(index)", kind: .agent(AgentTurn(blocks: blocks, stop: .end_turn), index: index, scope: "s:\(index)",
                                                      running: false, last: false, settings: nil, retry: false, subagents: []), exchange: 0)
    }
    let spinner = ThreadItem(id: ThreadItem.earlierID, kind: .earlier(loading: false), exchange: 0)
    coordinator.update(items: [spinner, agent(3, blocks: 1)], earlier: ViewEarlier(turns: 3, blocks: 9), environment: environment, in: view)
    try await Task.sleep(for: .milliseconds(200))
    view.layoutIfNeeded()
    #expect(view.contentSize.height < view.room && fills > 0 && asks == 0)

    // The spinner shows only while a part is on its way; the row is the same row at the same height either way
    let idle = try #require(view.collectionViewLayout.layoutAttributesForItem(at: IndexPath(item: 0, section: 0))).frame
    let loading = ThreadItem(id: ThreadItem.earlierID, kind: .earlier(loading: true), exchange: 0)
    coordinator.update(items: [loading, agent(3, blocks: 1)], earlier: ViewEarlier(turns: 3, blocks: 9), environment: environment, in: view)
    view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(100))
    view.layoutIfNeeded()
    #expect(try #require(view.collectionViewLayout.layoutAttributesForItem(at: IndexPath(item: 0, section: 0))).frame == idle)
    #expect(idle.height > 20)

    // A part that changes no row leaves the thread as short: it asks again
    var before = fills
    coordinator.update(items: [spinner, agent(3, blocks: 1)], earlier: ViewEarlier(turns: 3, blocks: 8), environment: environment, in: view)
    view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(50))
    #expect(fills > before)

    // The reader pulls down at the top (the jump stands in for the finger): one ask a pull
    _ = coordinator.scrollViewShouldScrollToTop(view)
    view.contentOffset.y = -116 - 30
    view.contentOffset.y = -116 - 60
    #expect(asks == 1)
    coordinator.scrollViewWillBeginDragging(view)
    view.contentOffset.y = -116 - 80
    // Not the reader's step: the finger is not down in a test
    #expect(asks == 1)
    _ = coordinator.scrollViewShouldScrollToTop(view)
    view.contentOffset.y = -116 - 40
    coordinator.scrollViewDidScrollToTop(view)
    #expect(asks == 2)

    // The part that makes the rows fill the room is the last the thread asks for: the reader's scrolling takes over
    before = fills
    coordinator.update(items: [spinner, agent(3, blocks: 12)], earlier: ViewEarlier(turns: 3, blocks: 0), environment: environment, in: view)
    view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(200))
    view.layoutIfNeeded()
    #expect(view.contentSize.height > view.room)
    coordinator.update(items: [spinner, agent(3, blocks: 12)], earlier: ViewEarlier(turns: 2, blocks: 4), environment: environment, in: view)
    view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(100))
    #expect(fills == before)
  }

  /// A jump to the top rests on the spinner itself; the part that makes the view whole takes that row away and puts
  /// rows taller than the screen in its place
  @Test func theLastPartLandsAboveAReaderRestingOnTheSpinner() async throws {
    let size = NSCollectionLayoutSize(widthDimension: .fractionalWidth(1), heightDimension: .estimated(120))
    let group = NSCollectionLayoutGroup.vertical(layoutSize: size, subitems: [NSCollectionLayoutItem(layoutSize: size)])
    let view = TranscriptCollectionView(frame: CGRect(x: 0, y: 0, width: 402, height: 874),
                                        collectionViewLayout: UICollectionViewCompositionalLayout(section: NSCollectionLayoutSection(group: group)))
    view.contentInsetAdjustmentBehavior = .never
    view.contentInset = UIEdgeInsets(top: 116, left: 0, bottom: 222, right: 0)
    view.motion = false
    let controller = UIViewController()
    controller.view.addSubview(view)
    let window = UIWindow(frame: view.frame)
    window.rootViewController = controller
    window.makeKeyAndVisible()
    defer { window.isHidden = true; window.rootViewController = nil }
    let coordinator = TranscriptCollection.Coordinator(control: TranscriptControl())
    coordinator.connect(view)
    var environment = EnvironmentValues()
    environment.hostedTranscript = true
    environment.motionEnabled = false
    func agent(_ index: Int) -> ThreadItem {
      let blocks = (0..<12).map { AgentBlock.text(TextBlock(markdown: "\(index).\($0): " + String(repeating: "measured reply text ", count: 30))) }
      return ThreadItem(id: "a\(index)", kind: .agent(AgentTurn(blocks: blocks, stop: .end_turn), index: index, scope: "s:\(index)",
                                                      running: false, last: false, settings: nil, retry: false, subagents: []), exchange: 0)
    }
    func prompt(_ index: Int) -> ThreadItem {
      ThreadItem(id: "u\(index)", kind: .user(UserTurn(text: String(repeating: "A measured prompt. ", count: 300)), index: index,
                                              editable: false, goal: false), exchange: index)
    }
    let spinner = ThreadItem(id: ThreadItem.earlierID, kind: .earlier(loading: false), exchange: 0)
    let tail = [prompt(2), agent(3), prompt(4), agent(5)]
    coordinator.update(items: [spinner] + tail, earlier: ViewEarlier(turns: 2, blocks: 0), environment: environment, in: view)
    try await Task.sleep(for: .milliseconds(200))
    view.layoutIfNeeded()
    view.maintainsBottomAnchor = false
    view.contentOffset.y = -116
    view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(100))
    view.layoutIfNeeded()
    func place(_ id: String) throws -> CGRect {
      let indexPath = try #require(view.readerIndexPath?(id))
      let frame = try #require(view.collectionViewLayout.layoutAttributesForItem(at: indexPath)).frame
      return frame.offsetBy(dx: 0, dy: -view.contentOffset.y)
    }
    let before = try place("u2")
    #expect(before.minY > 116 && before.minY < 300 && before.height > 874)

    coordinator.update(items: [prompt(0), agent(1)] + tail, earlier: nil, environment: environment, in: view)
    view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(200))
    view.layoutIfNeeded()
    #expect(abs(try place("u2").maxY - before.maxY) <= 0.5)
    #expect(try place("a1").height > 874)
  }

  private func selectionViews(in view: UIView) -> [SelectionTextView] {
    if let text = view as? SelectionTextView { return [text] }
    return view.subviews.flatMap { selectionViews(in: $0) }
  }

  private final class SingleRow: NSObject, UICollectionViewDataSource {
    func collectionView(_ collectionView: UICollectionView, numberOfItemsInSection section: Int) -> Int { 1 }
    func collectionView(_ collectionView: UICollectionView, cellForItemAt indexPath: IndexPath) -> UICollectionViewCell {
      collectionView.dequeueReusableCell(withReuseIdentifier: "row", for: indexPath)
    }
  }
}
#endif
