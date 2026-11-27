#if os(iOS)
import AcpiraDesign
import AcpiraMarkdown
import AcpiraModels
import SwiftUI
import UIKit
import WebKit

// The thread's scrolling on a UICollectionView, one hosted SwiftUI row per cell. A SwiftUI ScrollView over a
// LazyVStack shifts its offset whenever off-screen rows trade estimated heights for real ones, and the offset rules
// layered on top of it (keep the end, keep the top, park a reopened turn) fought those shifts: the thread jumped when
// the keyboard rose, when a turn started or ended, and while reading history above a running goal.
//
// The bottom anchoring, the reader anchor and the glide follow T3 Code's SwiftUI client (pingdotgg/t3code, MIT,
// `apps/swift-ios/Features/Chat/ThreadDetailView.swift`: `BottomAnchoredTranscriptCollectionView`,
// `TranscriptViewportGeometry`): the follow is a flag the reader's own drag releases and a drag ending near the end
// sets again; viewport changes (the keyboard, the docks) re-pin the end in the layout pass that applies them; rows the
// reader can see stay where they are when rows above them resize.

/// What the thread hands the collection view's owner: the jump back to the end, the fold hold, and the flags the
/// SwiftUI side observes
@MainActor final class TranscriptControl {
  weak var view: TranscriptCollectionView? {
    didSet { view?.safeFrame = safeFrame }
  }
  private var safeFrame = CGRect.zero

  /// Viewport animation changes clearance, not row content. Keep it off the cell reconfiguration path.
  func updateViewport(_ safe: CGRect, animated: Bool) {
    guard safeFrame != safe else { return }
    safeFrame = safe
    view?.safeFrame = safe
    view?.dockGlides = animated
    view?.setNeedsLayout()
  }
  /// More than half the room above the end (`ScrollToLatest`)
  let far = ObservedFlag()
  /// The thread follows its end (`threadFollowing`)
  let following = ObservedFlag(true)
  /// The reader's finger or its fling moves the thread (`scrollGesture`)
  let gesture = ScrollGesture()
  /// Folds opened or closed by hand, read back when a reused cell builds a row again
  let folds = FoldMemory()
  /// The older reply whose action row a tap has opened
  let reveal = TurnReveal()
  /// Keep the closure's identity across streamed pushes; a new one invalidates every hosted fold's environment.
  lazy var holdScroll = HoldScroll { [weak self] in self?.hold() }

  func toLatest(animated: Bool) {
    view?.followBottom(animated: animated)
  }

  /// A fold about to open or close by hand: the end lets go so the tapped row stays under the finger while the fold
  /// grows or shrinks below it (the reader anchor keeps the rows on screen in place). This holds while a running turn
  /// is followed too, as on the desktop (useBottomFollow.ts: a fold opened by hand never moves the view): kept pinned,
  /// the end pushed the tapped row and everything above it up by the fold's height, and back down on the close. Once
  /// the fold has settled with the end back in view, the thread follows it again (`followIfAtEndAfterFold`)
  func hold() {
    guard let view else { return }
    view.releaseReaderAnchor()
    view.maintainsBottomAnchor = false
    view.followIfAtEndAfterFold()
  }
}

/// A separate bridge for the bars' and docks' clearance. Animated safe-area changes must not reconfigure hosted rows.
struct TranscriptViewport: UIViewRepresentable {
  let safe: CGRect
  let control: TranscriptControl

  func makeUIView(context: Context) -> UIView {
    let view = UIView()
    view.isUserInteractionEnabled = false
    return view
  }

  func updateUIView(_ view: UIView, context: Context) {
    // Preserve the transaction so an animated dock glides, while a typed line still moves at once.
    control.updateViewport(safe, animated: context.transaction.animation != nil && !context.transaction.disablesAnimations)
  }
}

/// The thread's rows, updated only by content or environment changes, independently of viewport animation.
struct TranscriptCollection: UIViewRepresentable {
  let items: [ThreadItem]
  let control: TranscriptControl
  var liveSplits: LiveReplySplits? = nil
  var motion: Bool
  /// What the view leaves out ahead of its first row (`SessionView.earlier`), and the asks for it, of which one is under
  /// way at a time: the rows start with a spinner, the reader's scrolling near it asks (`AppModel.loadEarlier`), and a
  /// thread too short to scroll asks by itself (`AppModel.fillEarlier`)
  var earlier: ViewEarlier?
  var loadEarlier: (() -> Void)?
  var fillEarlier: (() -> Void)?
  /// The thread's own environment values on top of the representable's (`applyThread`, the scroll hooks)
  var decorate: (inout EnvironmentValues) -> Void

  func makeCoordinator() -> Coordinator { Coordinator(control: control) }

  func makeUIView(context: Context) -> TranscriptCollectionView {
    let view = TranscriptCollectionView(frame: .zero, collectionViewLayout: Self.layout())
    view.backgroundColor = .clear
    view.alwaysBounceVertical = true
    view.showsVerticalScrollIndicator = false
    view.showsHorizontalScrollIndicator = false
    // As in ChatGPT and Messages: the thread scrolls freely above an open keyboard, and only a drag that carries on into
    // the keyboard pulls it down with the finger
    view.keyboardDismissMode = .interactive
    view.delaysContentTouches = false
    // The insets come from SwiftUI's safe area (the bars and docks), so the system adds nothing of its own
    view.contentInsetAdjustmentBehavior = .never
    // The page's own fade covers the top (`TopFade`); the composer's glass covers the bottom
    // (iOS 26 edge effects; iOS 18 draws none)
    if #available(iOS 26, *) {
      view.topEdgeEffect.isHidden = true
      view.bottomEdgeEffect.isHidden = true
    }
    context.coordinator.connect(view)
    return view
  }

  func updateUIView(_ view: TranscriptCollectionView, context: Context) {
    var values = context.environment
    decorate(&values)
    // This belongs in the copied environment: an outer override would be replaced by the cell's `\.self` below.
    values.hostedTranscript = true
    control.view = view
    view.loadEarlier = earlier == nil ? nil : loadEarlier
    view.fillEarlier = earlier == nil ? nil : fillEarlier
    view.motion = motion
    context.coordinator.update(items: items, earlier: earlier, anchorRemap: liveSplits?.pendingAnchorRemap,
                               environment: values, in: view)
    liveSplits?.pendingAnchorRemap = nil
  }

  static func dismantleUIView(_ view: TranscriptCollectionView, coordinator: Coordinator) {
    view.stopFollowing()
  }

  private static func layout() -> UICollectionViewLayout {
    let size = NSCollectionLayoutSize(widthDimension: .fractionalWidth(1),
                                      heightDimension: .estimated(TranscriptCollectionView.estimatedRowHeight))
    let group = NSCollectionLayoutGroup.vertical(layoutSize: size, subitems: [NSCollectionLayoutItem(layoutSize: size)])
    let section = NSCollectionLayoutSection(group: group)
    // ThreadContent's own top and bottom padding; the rows carry the gaps between them
    section.contentInsets = NSDirectionalEdgeInsets(top: Metrics.padY, leading: 0, bottom: WebMetrics.msgGap, trailing: 0)
    return UICollectionViewCompositionalLayout(section: section)
  }

  /// Snapshot of environment values thread rows actually observe. When unchanged across streamed chunks, visible
  /// rows whose own `ThreadItem` did not change skip reconfiguration completely.
  private struct ThreadEnvFingerprint: Equatable {
    var colorScheme: ColorScheme
    var motionEnabled: Bool
    var processPrefs: ProcessPrefs
    var quietEntrance: Bool
    var planContext: PlanContext
    var widgetCanSend: Bool
    var allSubagents: [SubagentSummary]
    var sidekickModels: SidekickModels
    var chatAgent: AgentInfo?
    var syncing: Bool
    var liveStage: String?
    var canStopCommands: Bool
    var promptCommands: [String]
    var mentionPersonas: [MentionPersona]
    var compactAt: Int?
    var hiddenModels: [String: [String]]
    var clockOffset: TimeInterval
    var clockHeld: Date?
    var dynamicTypeSize: DynamicTypeSize
    var legibilityWeight: LegibilityWeight?
    var layoutDirection: LayoutDirection
    var accessibilityReduceMotion: Bool
    var accessibilityVoiceOverEnabled: Bool
    var displayScale: CGFloat
    var horizontalSizeClass: UserInterfaceSizeClass?
    var subagentTranscript: Bool
    var initialPopup: String?
    var media: MediaScope
    var liveSplitsID: ObjectIdentifier?

    init(_ env: EnvironmentValues) {
      colorScheme = env.colorScheme
      motionEnabled = env.motionEnabled
      processPrefs = env.processPrefs
      quietEntrance = env.quietEntrance
      planContext = env.planContext
      widgetCanSend = env.widgetCanSend
      allSubagents = env.allSubagents
      sidekickModels = env.sidekickModels
      chatAgent = env.chatAgent
      syncing = env.syncing
      liveStage = env.liveStage
      canStopCommands = env.canStopCommands
      promptCommands = env.promptCommands
      mentionPersonas = env.mentionPersonas
      compactAt = env.compactAt
      hiddenModels = env.hiddenModels
      clockOffset = env.clockOffset
      clockHeld = env.clockHeld
      dynamicTypeSize = env.dynamicTypeSize
      legibilityWeight = env.legibilityWeight
      layoutDirection = env.layoutDirection
      accessibilityReduceMotion = env.accessibilityReduceMotion
      accessibilityVoiceOverEnabled = env.accessibilityVoiceOverEnabled
      displayScale = env.displayScale
      horizontalSizeClass = env.horizontalSizeClass
      subagentTranscript = env.subagentTranscript
      initialPopup = env.initialPopup
      media = env.media
      liveSplitsID = env.liveReplySplits.map(ObjectIdentifier.init)
    }
  }

  @MainActor final class Coordinator: NSObject, UICollectionViewDelegate, UIGestureRecognizerDelegate,
                                      UIContextMenuInteractionDelegate {
    private let control: TranscriptControl
    private weak var collectionView: TranscriptCollectionView?
    private var dataSource: UICollectionViewDiffableDataSource<Int, String>?
    private var items: [String: ThreadItem] = [:]
    private var ids: [String] = []
    private var lastPromptID: String?
    private var queuedCount = 0
    private var earlier: ViewEarlier?
    private var lastEnvFingerprint: ThreadEnvFingerprint?
    /// Per-row block hit registries (populated on layout by `BlockStackLayout` without adding any per-block views)
    private var hitTargets: [String: RowHitTargets] = [:]
    /// The one row currently hosting on-demand native `SelectionTextView`s; nil during idle scrolling and streaming
    private var interactiveRowID: String?
    private var doubleTapGesture: UITapGestureRecognizer?
    /// Pressed block geometry for the open `UIContextMenuInteraction` so highlight and dismissal target that block
    /// instead of `TranscriptCollectionView` itself (which otherwise hides the whole transcript behind black)
    private struct ContextMenuTarget {
      let identifier: NSString
      let rowID: String
      let frameInCell: CGRect
      let fallbackCenter: CGPoint
    }
    private var activeContextMenu: ContextMenuTarget?
    /// The environment the rows read. Hosted cells do not inherit the SwiftUI environment across the representable,
    /// so it is copied in whole and handed to every cell a pass configures
    private var environment = EnvironmentValues()

    init(control: TranscriptControl) { self.control = control }

    private func hitTarget(for id: String) -> RowHitTargets {
      if let existing = hitTargets[id] { return existing }
      let created = RowHitTargets()
      hitTargets[id] = created
      return created
    }

    func connect(_ view: TranscriptCollectionView) {
      collectionView = view
      view.delegate = self
      // One registration per row kind, so a recycled cell usually shows the same kind of row and SwiftUI updates its
      // tree instead of rebuilding it (T3 Code's per-kind pools)
      let make = { [weak self] (resetIdentity: Bool) in
        UICollectionView.CellRegistration<UICollectionViewCell, String> { cell, _, id in
          guard let self, let item = self.items[id] else {
            cell.contentConfiguration = nil
            return
          }
          let environment = self.environment
          let targets = self.hitTarget(for: id)
          let active = self.interactiveRowID == id
          // Plain prose slices carry no per-row state and reuse their SwiftUI view graph in place during fast flings;
          // stateful rows (folds, streaming pacers, tables, code, images) reset identity when reused for another row.
          cell.contentConfiguration = UIHostingConfiguration {
            if resetIdentity {
              HostedRow(item: item, environment: environment, hitTargets: targets, selectionActive: active,
                        onSelectionEnd: { [weak self] in self?.deactivateSelection(id) })
                .id(id)
            } else {
              HostedRow(item: item, environment: environment, hitTargets: targets, selectionActive: active,
                        onSelectionEnd: { [weak self] in self?.deactivateSelection(id) })
            }
          }
          .margins(.all, 0)
          cell.backgroundConfiguration = .clear()
        }
      }
      let user = make(true), agent = make(true), plainPiece = make(false), heavyPiece = make(true), other = make(true)
      dataSource = UICollectionViewDiffableDataSource(collectionView: view) { [weak self] view, indexPath, id in
        let registration: UICollectionView.CellRegistration<UICollectionViewCell, String>
        if id.hasPrefix("u") { registration = user }
        else if let item = self?.items[id], item.isReplyPieceRow {
          registration = item.isPlainReplyPiece ? plainPiece : heavyPiece
        } else if id.hasPrefix("a") { registration = agent }
        else { registration = other }
        return view.dequeueConfiguredReusableCell(using: registration, for: indexPath, item: id)
      }
      view.readerItemID = { [weak self] in self?.dataSource?.itemIdentifier(for: $0) }
      view.readerIndexPath = { [weak self] in self?.dataSource?.indexPath(for: $0) }
      view.onFollowChange = { [weak self] in self?.control.following.set($0) }
      // One shared double-tap recognizer to mount native range selection on demand
      let doubleTap = UITapGestureRecognizer(target: self, action: #selector(handleDoubleTap(_:)))
      doubleTap.numberOfTapsRequired = 2
      doubleTap.cancelsTouchesInView = false
      doubleTap.delegate = self
      view.addGestureRecognizer(doubleTap)
      doubleTapGesture = doubleTap
      // A tap on the thread ends typing and toggles an older reply's action row (waiting out double tap);
      // transcript buttons and links still take the same tap
      let tap = UITapGestureRecognizer(target: self, action: #selector(handleSingleTap(_:)))
      tap.cancelsTouchesInView = false
      tap.delegate = self
      tap.require(toFail: doubleTap)
      view.addGestureRecognizer(tap)
      // One shared UIContextMenuInteraction replaces per-paragraph SwiftUI `.contextMenu` modifiers
      view.addInteraction(UIContextMenuInteraction(delegate: self))
    }

    /// A pass lays out new and removed rows and reconfigures the rows whose data changed, plus visible rows only when
    /// the thread-wide environment fingerprint actually changed.
    func update(items rows: [ThreadItem], earlier window: ViewEarlier? = nil,
                anchorRemap: (from: String, to: String)? = nil,
                environment values: EnvironmentValues,
                in view: TranscriptCollectionView) {
      guard let dataSource else { return }
      environment = values
      let fingerprint = ThreadEnvFingerprint(values)
      let envChanged = fingerprint != lastEnvFingerprint
      lastEnvFingerprint = fingerprint
      let next = rows.map(\.id)
      let old = Set(ids)
      // The view starts elsewhere than it did: an earlier part landed ahead of the rows (or the engine opened the
      // session at its tail again)
      let moved = !ids.isEmpty && window != earlier
      earlier = window
      var refresh: Set<String> = envChanged
        ? Set(view.indexPathsForVisibleItems.compactMap { dataSource.itemIdentifier(for: $0) })
        : []
      for row in rows {
        if items[row.id] != row { refresh.insert(row.id) }
        items[row.id] = row
      }
      let initial = ids.isEmpty
      let promptID = rows.last { $0.id.hasPrefix("u") }?.id
      let nowQueued: Int = if case .queue(let q, _) = rows.last?.kind { q.count } else { 0 }
      // A newly sent or queued user prompt brings the reader back to the tail (useBottomFollow.ts `lastPrompt`),
      // including after tapping a widget button higher up; automatic goal turns carry no `u` row and stay put
      let sentNow = !initial && ((promptID != nil && promptID != lastPromptID && !old.contains(promptID!)) || nowQueued > queuedCount)
      lastPromptID = promptID
      queuedCount = nowQueued
      if sentNow {
        view.releaseReaderAnchor()
        view.maintainsBottomAnchor = true
      }
      let structure = next != ids
      if structure {
        for id in old.subtracting(next) {
          items[id] = nil
          hitTargets[id] = nil
          if interactiveRowID == id { interactiveRowID = nil }
        }
        ids = next
      }
      // Only rows that stay can be reconfigured; new ones are configured as they are dequeued
      refresh = refresh.intersection(old).intersection(next)
      // A part can land without a row changing (blocks inside a closed fold): the pass that follows still finds out
      // whether the thread fills its room (`TranscriptCollectionView.short`)
      if moved { view.setNeedsLayout() }
      guard structure || !refresh.isEmpty else { return }
      var snapshot = NSDiffableDataSourceSnapshot<Int, String>()
      snapshot.appendSections([0])
      snapshot.appendItems(next)
      if !refresh.isEmpty { snapshot.reconfigureItems(Array(refresh)) }
      // Rows the reader can see stay put through rows arriving, leaving or growing; reaching new output is the
      // follow's job. An earlier part goes in above what the reader sees: new rows, and earlier blocks at the head of
      // the turn the view was cut in, which grows upwards. So that pass holds a row by its end, mid-gesture too (the
      // reader is usually scrolling up when it lands), and never by the spinner the part goes in under
      var visible = initial || sentNow ? nil : view.firstVisibleItem(skipping: moved ? ThreadItem.earlierID : nil)
      // When a newly frozen row is promoted directly above the live tail that currently anchors the viewport, remap
      // the anchor to that frozen row (whose top sits at the old live tail's top) so reading stays pixel-stable
      if let remap = anchorRemap, let v = visible, v.itemID == remap.from, items[remap.to] != nil {
        visible = TranscriptCollectionView.VisibleItem(itemID: remap.to, indexPath: v.indexPath,
                                                       minYInViewport: v.minYInViewport, maxYInViewport: v.maxYInViewport)
      }
      dataSource.apply(snapshot, animatingDifferences: false) { [weak view] in
        DispatchQueue.main.async {
          guard let view, view.maintainsBottomAnchor, !view.isTracking, !view.isDragging, !view.isDecelerating else { return }
          view.layoutIfNeeded()
          view.followBottom(animated: !initial)
        }
      }
      if let visible { moved ? view.keepEnd(visible) : view.keepOnScreen(visible) }
    }

    // MARK: - On-demand selection & context menu

    private enum SelectionMode { case selectAll, selectWord }

    /// Mounts native `SelectionTextView` overlays on `rowID` only, then begins range selection at `pointInCell`.
    private func activateSelection(rowID: String, pointInCell: CGPoint, mode: SelectionMode) {
      guard let view = collectionView, let dataSource, items[rowID] != nil else { return }
      let previous = interactiveRowID
      interactiveRowID = rowID
      var toReconfigure: [String] = [rowID]
      if let previous, previous != rowID, items[previous] != nil { toReconfigure.append(previous) }
      var snapshot = dataSource.snapshot()
      snapshot.reconfigureItems(toReconfigure.filter { snapshot.indexOfItem($0) != nil })
      dataSource.apply(snapshot, animatingDifferences: false)
      view.layoutIfNeeded()

      guard let indexPath = dataSource.indexPath(for: rowID),
            let cell = view.cellForItem(at: indexPath),
            let target = nearestSelectionView(in: cell, to: pointInCell) else { return }
      let localPoint = cell.contentView.convert(pointInCell, to: target)
      switch mode {
      case .selectAll:
        DispatchQueue.main.async { [weak target] in target?.beginSelection() }
      case .selectWord:
        target.beginWordSelection(at: localPoint)
      }
    }

    /// Unmounts on-demand `SelectionTextView` overlays once the user dismisses selection.
    private func deactivateSelection(_ rowID: String) {
      guard interactiveRowID == rowID else { return }
      interactiveRowID = nil
      guard dataSource != nil else { return }
      // Defer past `resignFirstResponder` so UIKit finishes tearing down the edit menu before reconfiguring the cell
      DispatchQueue.main.async { [weak self] in
        guard let self, self.interactiveRowID == nil, let dataSource = self.dataSource else { return }
        var snapshot = dataSource.snapshot()
        guard snapshot.indexOfItem(rowID) != nil else { return }
        snapshot.reconfigureItems([rowID])
        dataSource.apply(snapshot, animatingDifferences: false)
      }
    }

    private func nearestSelectionView(in cell: UICollectionViewCell, to pointInContent: CGPoint) -> SelectionTextView? {
      var all: [SelectionTextView] = []
      func collect(_ v: UIView) {
        if let tv = v as? SelectionTextView { all.append(tv); return }
        for sub in v.subviews { collect(sub) }
      }
      collect(cell.contentView)
      guard !all.isEmpty else { return nil }
      // Prefer the view whose frame contains the point, else the vertically nearest one
      if let exact = all.first(where: { $0.convert($0.bounds, to: cell.contentView).contains(pointInContent) }) {
        return exact
      }
      return all.min { a, b in
        let fa = a.convert(a.bounds, to: cell.contentView)
        let fb = b.convert(b.bounds, to: cell.contentView)
        let da = hypot(max(0, max(fa.minX - pointInContent.x, pointInContent.x - fa.maxX)),
                       max(0, max(fa.minY - pointInContent.y, pointInContent.y - fa.maxY)))
        let db = hypot(max(0, max(fb.minX - pointInContent.x, pointInContent.x - fb.maxX)),
                       max(0, max(fb.minY - pointInContent.y, pointInContent.y - fb.maxY)))
        return da < db
      }
    }

    private func hitBlock(at locationInCollection: CGPoint) -> (id: String, cell: UICollectionViewCell,
                                                               pointInCell: CGPoint, entry: BlockHitEntry,
                                                               frameInCell: CGRect)? {
      guard let view = collectionView, let dataSource,
            let indexPath = view.indexPathForItem(at: locationInCollection),
            let id = dataSource.itemIdentifier(for: indexPath),
            let cell = view.cellForItem(at: indexPath),
            let targets = hitTargets[id] else { return nil }
      let pointInCell = view.convert(locationInCollection, to: cell.contentView)
      guard let hit = targets.hit(at: pointInCell) else { return nil }
      return (id, cell, pointInCell, hit.entry, hit.frameInRow)
    }

    @objc private func handleDoubleTap(_ tap: UITapGestureRecognizer) {
      guard tap.state == .ended, let view = collectionView else { return }
      let loc = tap.location(in: view)
      guard let hit = hitBlock(at: loc), hit.entry.offersSelection else { return }
      if interactiveRowID == hit.id { return }
      activateSelection(rowID: hit.id, pointInCell: hit.pointInCell, mode: .selectWord)
    }

    @objc private func handleSingleTap(_ tap: UITapGestureRecognizer) {
      guard tap.state == .ended, let view = collectionView else { return }
      UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
      let loc = tap.location(in: view)
      guard let hit = hitBlock(at: loc),
            let targets = hitTargets[hit.id],
            targets.canTapActions, let turnIndex = targets.turnIndex else { return }
      // Leave taps on links alone so `openURL` handles them without flipping the action row
      let localPoint = CGPoint(x: hit.pointInCell.x - hit.frameInCell.minX,
                               y: hit.pointInCell.y - hit.frameInCell.minY)
      if TextSelectionProbe.anyLink(in: hit.entry.renderedBlock, width: hit.frameInCell.width,
                                    pointInBlock: localPoint) != nil {
        return
      }
      control.reveal.toggle(turnIndex)
    }

    func contextMenuInteraction(_ interaction: UIContextMenuInteraction,
                                configurationForMenuAtLocation location: CGPoint) -> UIContextMenuConfiguration? {
      guard !environment.subagentTranscript else { return nil }
      guard let hit = hitBlock(at: location) else { return nil }
      // While native range selection is active on this row, leave gestures to `SelectionTextView`
      if interactiveRowID == hit.id { return nil }
      let localPoint = CGPoint(x: hit.pointInCell.x - hit.frameInCell.minX,
                               y: hit.pointInCell.y - hit.frameInCell.minY)
      let inspection = TextSelectionProbe.inspect(block: hit.entry.renderedBlock, quoteBlock: hit.entry.quoteBlock,
                                                  width: hit.frameInCell.width, pointInBlock: localPoint)
      let quoteText = QuoteText.of(hit.entry.quoteBlock)
      let quoteAction = environment.sessionActions.quote
      let offersSelection = hit.entry.offersSelection
      let rowID = hit.id
      let pointInCell = hit.pointInCell
      let scheme = collectionView?.traitCollection.userInterfaceStyle == .light ? ColorScheme.light : .dark

      let identifier = UUID().uuidString as NSString
      let fallbackCenter = collectionView.map { view in
        hit.cell.contentView.convert(CGPoint(x: hit.frameInCell.midX, y: hit.frameInCell.midY), to: view)
      } ?? location
      activeContextMenu = ContextMenuTarget(identifier: identifier, rowID: rowID,
                                            frameInCell: hit.frameInCell, fallbackCenter: fallbackCenter)

      let previewProvider: UIContextMenuContentPreviewProvider? = offersSelection ? {
        let card = QuotePreviewCard(text: inspection.previewText)
          .environment(\.colorScheme, scheme)
        let host = UIHostingController(rootView: card)
        host.view.backgroundColor = .clear
        let size = host.sizeThatFits(in: CGSize(width: Metrics.popLg + Metrics.s3 * 2, height: 600))
        host.preferredContentSize = size
        return host
      } : nil

      return UIContextMenuConfiguration(identifier: identifier, previewProvider: previewProvider) { [weak self] _ in
        var items: [UIMenuElement] = []
        if let link = inspection.link {
          let openSystem = UIAction(title: t("browser.openSystem"), image: UIImage(systemName: "safari")) { _ in
            WebPage.openOutside(link)
          }
          let copyLink = UIAction(title: t("mobile.link.copy"), image: UIImage(systemName: "link")) { _ in
            UIPasteboardBridge.copy(link.absoluteString)
          }
          items.append(UIMenu(title: link.absoluteString, options: .displayInline, children: [openSystem, copyLink]))
        }
        items.append(UIAction(title: t("quote.add"), image: UIImage(systemName: "text.quote")) { _ in
          quoteAction(quoteText)
        })
        items.append(UIAction(title: t("common.copy"), image: UIImage(systemName: "doc.on.doc")) { _ in
          UIPasteboardBridge.copy(quoteText)
        })
        if offersSelection {
          items.append(UIAction(title: t("history.selectText"), image: UIImage(systemName: "text.cursor")) { [weak self] _ in
            DispatchQueue.main.async {
              self?.activateSelection(rowID: rowID, pointInCell: pointInCell, mode: .selectAll)
            }
          })
        }
        return UIMenu(children: items)
      }
    }

    func contextMenuInteraction(_ interaction: UIContextMenuInteraction,
                                configuration: UIContextMenuConfiguration,
                                highlightPreviewForItemWithIdentifier identifier: any NSCopying) -> UITargetedPreview? {
      targetedPreview(for: identifier)
    }

    func contextMenuInteraction(_ interaction: UIContextMenuInteraction,
                                configuration: UIContextMenuConfiguration,
                                dismissalPreviewForItemWithIdentifier identifier: any NSCopying) -> UITargetedPreview? {
      targetedPreview(for: identifier)
    }

    func contextMenuInteraction(_ interaction: UIContextMenuInteraction,
                                willEndFor configuration: UIContextMenuConfiguration,
                                animator: (any UIContextMenuInteractionAnimating)?) {
      let id = configuration.identifier
      let clear = { [weak self] in
        if self?.activeContextMenu?.identifier.isEqual(id) == true {
          self?.activeContextMenu = nil
        }
      }
      if let animator {
        animator.addCompletion(clear)
      } else {
        clear()
      }
    }

    /// Targets an unparented snapshot of the pressed block rather than `TranscriptCollectionView` itself:
    /// UIKit hides the targeted preview's view during highlight and dismissal, so falling back to `interaction.view`
    /// blanked out the entire transcript to solid black until the menu finished dismissing.
    private func targetedPreview(for identifier: any NSCopying) -> UITargetedPreview? {
      guard let target = activeContextMenu, target.identifier.isEqual(identifier),
            let view = collectionView, view.window != nil else { return nil }
      let cell = dataSource?.indexPath(for: target.rowID).flatMap { view.cellForItem(at: $0) }
      let visibleRect: CGRect?
      if let cell {
        let raw = cell.contentView.convert(target.frameInCell, to: view)
        let clipped = raw.intersection(view.bounds)
        visibleRect = !clipped.isNull && clipped.width >= 1 && clipped.height >= 1 ? clipped : nil
      } else {
        visibleRect = nil
      }
      let center: CGPoint
      if let visibleRect {
        center = CGPoint(x: visibleRect.midX, y: visibleRect.midY)
      } else {
        center = CGPoint(x: min(max(target.fallbackCenter.x, view.bounds.minX), view.bounds.maxX),
                         y: min(max(target.fallbackCenter.y, view.bounds.minY), view.bounds.maxY))
      }
      let previewTarget = UIPreviewTarget(container: view, center: center)
      let params = UIPreviewParameters()
      params.backgroundColor = .clear
      if let cell, let visibleRect {
        let rectInCell = view.convert(visibleRect, to: cell.contentView)
        if rectInCell.width >= 1, rectInCell.height >= 1,
           let snapshot = cell.contentView.resizableSnapshotView(from: rectInCell, afterScreenUpdates: false,
                                                                 withCapInsets: .zero) {
          params.visiblePath = UIBezierPath(roundedRect: snapshot.bounds, cornerRadius: Metrics.rSm)
          return UITargetedPreview(view: snapshot, parameters: params, target: previewTarget)
        }
      }
      let size = visibleRect?.size ?? CGSize(width: 2, height: 2)
      let placeholder = UIView(frame: CGRect(origin: .zero, size: size))
      placeholder.backgroundColor = .clear
      params.shadowPath = UIBezierPath()
      return UITargetedPreview(view: placeholder, parameters: params, target: previewTarget)
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
      // The selectable transcript text and embedded HTML widgets own their taps and focus.
      var target = touch.view
      while let view = target {
        if view is SelectionTextView || view is WKWebView { return false }
        target = view.superview
      }
      return true
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer,
                           shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }

    func scrollViewWillBeginDragging(_ scrollView: UIScrollView) {
      control.gesture.active = true
      guard let view = scrollView as? TranscriptCollectionView else { return }
      view.releaseReaderAnchor()
      view.maintainsBottomAnchor = false
      view.readerJump = false
      view.pulled = false
    }

    func scrollViewShouldScrollToTop(_ scrollView: UIScrollView) -> Bool {
      (scrollView as? TranscriptCollectionView)?.releaseReaderAnchor()
      (scrollView as? TranscriptCollectionView)?.maintainsBottomAnchor = false
      (scrollView as? TranscriptCollectionView)?.readerJump = true
      return true
    }

    func scrollViewDidScrollToTop(_ scrollView: UIScrollView) { (scrollView as? TranscriptCollectionView)?.readerJump = false }

    func scrollViewDidEndScrollingAnimation(_ scrollView: UIScrollView) {
      (scrollView as? TranscriptCollectionView)?.readerJump = false
    }

    func scrollViewDidEndDragging(_ scrollView: UIScrollView, willDecelerate decelerate: Bool) {
      guard !decelerate else { return }
      settle(scrollView)
    }

    func scrollViewDidEndDecelerating(_ scrollView: UIScrollView) { settle(scrollView) }

    func scrollViewDidScroll(_ scrollView: UIScrollView) {
      guard let view = scrollView as? TranscriptCollectionView else { return }
      control.far.set(view.distanceToEnd > view.room / 2)
      view.readerScrolled()
    }

    /// The gesture is over: a thread let go near its end follows it again
    private func settle(_ scrollView: UIScrollView) {
      control.gesture.active = false
      guard let view = scrollView as? TranscriptCollectionView else { return }
      view.maintainsBottomAnchor = view.distanceToEnd < TranscriptCollectionView.followDistance
      view.setNeedsLayout()
    }
  }
}

/// The scroll view's end, the room the insets leave, and the moves that keep the end in view (T3 Code's
/// `TranscriptViewportGeometry`)
struct TranscriptViewportGeometry: Equatable {
  let contentHeight: CGFloat
  let viewportHeight: CGFloat
  let topInset: CGFloat
  let bottomInset: CGFloat

  var bottomOffset: CGFloat { max(-topInset, contentHeight - viewportHeight + bottomInset) }

  /// Where the end goes after a layout, while the thread follows it and no finger moves it; nil leaves the offset alone
  func restoredBottomOffset(after previous: Self?, maintainsBottomAnchor: Bool, isInteracting: Bool) -> CGFloat? {
    guard maintainsBottomAnchor, !isInteracting else { return nil }
    guard let previous, previous.contentHeight > 0, previous.viewportHeight > 0 else {
      return contentHeight > 0 && viewportHeight > 0 ? bottomOffset : nil
    }
    let contentChanged = abs(contentHeight - previous.contentHeight) > 0.5
    let viewportChanged = abs(viewportHeight - previous.viewportHeight) > 0.5 || abs(bottomInset - previous.bottomInset) > 0.5
    guard contentChanged || viewportChanged else { return nil }
    return bottomOffset
  }

  /// Only the dock moved: the bottom inset changed and nothing else did
  func onlyDockMoved(after previous: Self?) -> Bool {
    guard let previous, previous.contentHeight > 0 else { return false }
    return abs(viewportHeight - previous.viewportHeight) <= 0.5 && abs(topInset - previous.topInset) <= 0.5
      && abs(contentHeight - previous.contentHeight) <= 0.5 && abs(bottomInset - previous.bottomInset) > 0.5
  }

  /// Only content growth or shrink glides; a changed viewport (the keyboard, a dock) snaps with it
  func glidesToBottom(after previous: Self?) -> Bool {
    guard let previous, previous.contentHeight > 0 else { return false }
    return abs(viewportHeight - previous.viewportHeight) <= 0.5 && abs(topInset - previous.topInset) <= 0.5
      && abs(bottomInset - previous.bottomInset) <= 0.5
  }

  /// One step of a critically damped spring (exact solution, so any frame rate traces the same curve). It keeps its
  /// velocity when the next chunk moves the target, so back-to-back chunks read as one continuous scroll; from rest it
  /// covers 90% of the distance in about 280 ms
  static func glideStep(from current: CGFloat, velocity: CGFloat, to target: CGFloat,
                        elapsed: CFTimeInterval) -> (offset: CGFloat, velocity: CGFloat) {
    let omega: CGFloat = 14
    let dt = CGFloat(max(0, elapsed))
    let delta = current - target
    let decay = exp(-omega * dt)
    let nextDelta = (delta + (velocity + omega * delta) * dt) * decay
    let nextVelocity = (velocity - omega * (velocity + omega * delta) * dt) * decay
    if abs(nextDelta) <= 0.5, abs(nextVelocity) <= 20 { return (target, 0) }
    return (target + nextDelta, nextVelocity)
  }
}

/// A row under the thread's copied environment, except the colour scheme, which it takes from its own cell. A copy
/// is only as new as the pass that configured the cell, and a pass reconfigures the rows on screen alone: a row laid
/// out ahead of the viewport before the app's theme had replaced the system's at launch kept the system's scheme,
/// and drew a light bubble in a dark chat. The cell's traits follow the window, so they are never behind
private struct HostedRow: View {
  let item: ThreadItem
  let environment: EnvironmentValues
  let hitTargets: RowHitTargets
  let selectionActive: Bool
  let onSelectionEnd: () -> Void
  @Environment(\.colorScheme) private var scheme
  @State private var probe = CellSizeProbe()

  var body: some View {
    let base = ThreadItemView(item: item)
      .environment(\.colorScheme, scheme)
      .environment(\.deferSelectionSurface, true)
      .environment(\.selectionSurfaceActive, selectionActive)
      .environment(\.selectionEndHandler, selectionActive ? SelectionEndHandler(id: item.id, run: onSelectionEnd) : nil)
      .environment(\.deferQuoteMenu, true)
      .environment(\.rowHitTargets, hitTargets)
      .environment(\.self, environment)
      .coordinateSpace(.named(RowHitSpace.name))
    if item.needsSizeProbe {
      // Content growing in place inside a horizontal ScrollView (a streaming table or code block finishing its pacing
      // flush, or a live row) updates its SwiftUI frame without invalidating UIHostingConfiguration's self-sized cell
      // height. Plain settled text slices skip the probe UIViewRepresentable altogether.
      base
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { probe.heightChanged($0) }
        .background(CellSizeAnchor(probe: probe))
    } else {
      base
    }
  }
}

private extension ThreadItem {
  /// Whether this row renders directly as a `ReplyPieceView` (either a live frozen slice or a settled single-text piece).
  var isReplyPieceRow: Bool {
    switch kind {
    case .replyPiece:
      return true
    case .agent(_, _, _, false, _, _, _, _, .piece, _):
      return settledSlices?.count == 1
    default:
      return false
    }
  }

  /// Segments rendered by a `ReplyPieceView` row, if this row is one.
  private var pieceSegments: [ReplySegment]? {
    switch kind {
    case .replyPiece(let segments, _, _, _):
      return segments
    case .agent(_, _, _, false, _, _, _, _, .piece, _):
      return settledSlices?.count == 1 ? settledSlices?[0].segments : nil
    default:
      return nil
    }
  }

  /// Plain prose slice (paragraphs, headings, lists, blockquotes without tables, code, math, or embedded images):
  /// carries no per-row state and can reuse its SwiftUI view graph in place across `UICollectionView` cell dequeues.
  var isPlainReplyPiece: Bool {
    guard let segments = pieceSegments else { return false }
    return !segments.contains { seg in
      switch seg {
      case .block(let b, _):
        return Self.hasStatefulBlock(b)
      case .items(_, _, _, let items):
        return items.contains { $0.contains(where: Self.hasStatefulBlock) }
      }
    }
  }

  private static func hasStatefulBlock(_ block: MDBlock) -> Bool {
    switch block {
    case .table, .code, .math:
      return true
    case .paragraph(let inlines):
      return inlines.contains { if case .image = $0 { true } else { false } }
    case .quote(let inner):
      return inner.contains(where: hasStatefulBlock)
    case .list(_, _, let items):
      return items.contains { $0.contains(where: hasStatefulBlock) }
    case .heading, .rule:
      return false
    }
  }

  /// Whether this row can resize asynchronously inside a horizontal `ScrollView` (tables, fenced code, or live rows).
  var needsSizeProbe: Bool {
    switch kind {
    case .liveTail:
      return true
    case .replyPiece:
      return !isPlainReplyPiece
    case .agent(_, _, _, let running, _, _, _, _, let cut, _):
      if running { return true }
      switch cut {
      case .piece:
        return isReplyPieceRow ? !isPlainReplyPiece : true
      case .whole, .head, .tail:
        return true
      case .headOnly:
        return false
      }
    case .earlier, .user, .returns, .queue:
      return false
    }
  }
}
#endif
