#if os(iOS)
import AcpiraDesign
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
    context.coordinator.update(items: items, earlier: earlier, environment: values, in: view)
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

  @MainActor final class Coordinator: NSObject, UICollectionViewDelegate, UIGestureRecognizerDelegate {
    private let control: TranscriptControl
    private var dataSource: UICollectionViewDiffableDataSource<Int, String>?
    private var items: [String: ThreadItem] = [:]
    private var ids: [String] = []
    private var lastPromptID: String?
    private var queuedCount = 0
    private var earlier: ViewEarlier?
    /// The environment the rows read. Hosted cells do not inherit the SwiftUI environment across the representable,
    /// so it is copied in whole and handed to every cell a pass configures
    private var environment = EnvironmentValues()

    init(control: TranscriptControl) { self.control = control }

    func connect(_ view: TranscriptCollectionView) {
      view.delegate = self
      // One registration per row kind, so a recycled cell usually shows the same kind of row and SwiftUI updates its
      // tree instead of rebuilding it (T3 Code's per-kind pools)
      let make = {
        UICollectionView.CellRegistration<UICollectionViewCell, String> { [weak self] cell, _, id in
          guard let self, let item = self.items[id] else {
            cell.contentConfiguration = nil
            return
          }
          let environment = self.environment
          // `.id`: a cell reused for another row starts that row's state afresh; reconfiguring the same row keeps it
          cell.contentConfiguration = UIHostingConfiguration {
            HostedRow(item: item, environment: environment).id(id)
          }
          .margins(.all, 0)
          cell.backgroundConfiguration = .clear()
        }
      }
      let user = make(), agent = make(), other = make()
      dataSource = UICollectionViewDiffableDataSource(collectionView: view) { view, indexPath, id in
        let registration = id.hasPrefix("u") ? user : id.hasPrefix("a") ? agent : other
        return view.dequeueConfiguredReusableCell(using: registration, for: indexPath, item: id)
      }
      view.readerItemID = { [weak self] in self?.dataSource?.itemIdentifier(for: $0) }
      view.readerIndexPath = { [weak self] in self?.dataSource?.indexPath(for: $0) }
      view.onFollowChange = { [weak self] in self?.control.following.set($0) }
      // A tap on the thread ends typing; transcript buttons and links still take the same tap
      let tap = UITapGestureRecognizer(target: self, action: #selector(endTyping))
      tap.cancelsTouchesInView = false
      tap.delegate = self
      view.addGestureRecognizer(tap)
    }

    /// A pass lays out new and removed rows and reconfigures the rows whose data changed, plus the visible ones (the
    /// environment they read may have changed): a hosted row only redraws when its cell is configured again
    func update(items rows: [ThreadItem], earlier window: ViewEarlier? = nil, environment values: EnvironmentValues,
                in view: TranscriptCollectionView) {
      guard let dataSource else { return }
      environment = values
      let next = rows.map(\.id)
      let old = Set(ids)
      // The view starts elsewhere than it did: an earlier part landed ahead of the rows (or the engine opened the
      // session at its tail again)
      let moved = !ids.isEmpty && window != earlier
      earlier = window
      var refresh = Set(view.indexPathsForVisibleItems.compactMap { dataSource.itemIdentifier(for: $0) })
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
        for id in old.subtracting(next) { items[id] = nil }
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
      let visible = initial || sentNow ? nil : view.firstVisibleItem(skipping: moved ? ThreadItem.earlierID : nil)
      dataSource.apply(snapshot, animatingDifferences: false) { [weak view] in
        DispatchQueue.main.async {
          guard let view, view.maintainsBottomAnchor, !view.isTracking, !view.isDragging, !view.isDecelerating else { return }
          view.layoutIfNeeded()
          view.followBottom(animated: !initial)
        }
      }
      if let visible { moved ? view.keepEnd(visible) : view.keepOnScreen(visible) }
    }

    @objc private func endTyping() {
      UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
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
  @Environment(\.colorScheme) private var scheme
  @State private var probe = CellSizeProbe()

  var body: some View {
    ThreadItemView(item: item)
      .environment(\.colorScheme, scheme)
      .environment(\.self, environment)
      // Content growing in place inside a horizontal ScrollView (a streaming table or code block finishing its pacing
      // flush, or async inline-code chip growth) updates its SwiftUI frame without invalidating UIHostingConfiguration's
      // self-sized cell height. Keep the probe outside SwiftUI state so normal frames never re-evaluate this body.
      .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { probe.heightChanged($0) }
      .background(CellSizeAnchor(probe: probe))
  }
}

/// Invalidates the enclosing collection cell when SwiftUI's laid-out height diverges from the cell's bounds.
@MainActor private final class CellSizeProbe {
  weak var anchor: UIView?
  private var latest: CGFloat = 0
  private var lastInvalidated: CGFloat = 0
  private var pending = false

  func heightChanged(_ height: CGFloat) {
    guard height > 0 else { return }
    latest = height
    guard let cell = enclosingCell(), abs(cell.bounds.height - height) > 0.5 else {
      lastInvalidated = height
      return
    }
    guard !pending, abs(lastInvalidated - height) > 0.5 else { return }
    pending = true
    DispatchQueue.main.async { [weak self] in self?.flush() }
  }

  private func flush() {
    pending = false
    let height = latest
    guard height > 0, let (cell, collection) = enclosingCellAndCollection(),
          abs(cell.bounds.height - height) > 0.5 else {
      lastInvalidated = height
      return
    }
    lastInvalidated = height
    // The row's new height goes to the layout the way self-sizing hands it over, as this one row's preferred
    // attributes. `invalidateItems(at:)` made the compositional layout drop measured heights of other rows as well:
    // the content lost thousands of points in one frame, and a fling through history was thrown far from the reader's
    // place. The cell's own `invalidateIntrinsicContentSize` alone leaves a grown row at its old height.
    let layout = collection.collectionViewLayout
    guard let indexPath = collection.indexPath(for: cell),
          let original = layout.layoutAttributesForItem(at: indexPath) else { return }
    let preferred = cell.preferredLayoutAttributesFitting(original.copy() as! UICollectionViewLayoutAttributes)
    guard layout.shouldInvalidateLayout(forPreferredLayoutAttributes: preferred, withOriginalAttributes: original) else { return }
    layout.invalidateLayout(with: layout.invalidationContext(forPreferredLayoutAttributes: preferred,
                                                             withOriginalAttributes: original))
    collection.setNeedsLayout()
  }

  private func enclosingCell() -> UICollectionViewCell? {
    var parent = anchor?.superview
    while let view = parent {
      if let cell = view as? UICollectionViewCell { return cell }
      parent = view.superview
    }
    return nil
  }

  private func enclosingCellAndCollection() -> (UICollectionViewCell, UICollectionView)? {
    var parent = anchor?.superview
    var cell: UICollectionViewCell?
    while let view = parent {
      if cell == nil, let c = view as? UICollectionViewCell { cell = c }
      if let collection = view as? UICollectionView, let cell { return (cell, collection) }
      parent = view.superview
    }
    return nil
  }
}

private struct CellSizeAnchor: UIViewRepresentable {
  let probe: CellSizeProbe

  func makeUIView(context: Context) -> UIView {
    let view = UIView()
    view.isUserInteractionEnabled = false
    view.isHidden = true
    probe.anchor = view
    return view
  }

  func updateUIView(_ view: UIView, context: Context) {
    probe.anchor = view
  }
}

/// Self-sizing hosted rows change the thread's height after a snapshot lands, and the keyboard changes the viewport
/// without changing the content at all: the end is kept only while the reader follows it, and otherwise the rows on
/// screen are (T3 Code's `BottomAnchoredTranscriptCollectionView`)
final class TranscriptCollectionView: UICollectionView {
  // Bar and keyboard clearance already lives in contentInset. Passing UIKit's safe area on to hosted cells makes
  // their intrinsic height grow as they cross a bar, feeding their own scrolling position back into the layout.
  override var safeAreaInsets: UIEdgeInsets { .zero }

  var maintainsBottomAnchor = false {
    didSet {
      if maintainsBottomAnchor { pendingReaderAnchor = nil } else { stopBottomGlide() }
      if maintainsBottomAnchor != oldValue { onFollowChange?(maintainsBottomAnchor) }
    }
  }
  var onFollowChange: ((Bool) -> Void)?
  var readerItemID: ((IndexPath) -> String?)?
  var readerIndexPath: ((String) -> IndexPath?)?
  /// Glides follow streamed output; without motion the end snaps
  var motion = true
  /// The safe area's frame in the window. The view spans the bars and docks around it, and how far it reaches past it
  /// on each side is the content inset, worked out on every layout so the keyboard moving the frame lands in the same pass
  var safeFrame = CGRect.zero
  /// The last `safeFrame` change came in an animated transaction: a dock that only moved the bottom inset glides
  /// (`updateUIView`); taken by the next layout
  var dockGlides = false
  /// Ask for the part of the session ahead of the view, the reader's (`readerScrolled`) and the thread's own (`short`);
  /// one is under way at a time (`AppModel.loadEarlier`, `fillEarlier`). Nil when the view holds the session from its
  /// first turn; set, the first row is the spinner
  var loadEarlier: (() -> Void)?
  var fillEarlier: (() -> Void)?
  /// A jump the reader asked for is running (the status bar, a VoiceOver scroll): its scroll steps are the reader's, as
  /// a finger's and a fling's are (`readerScrolled`)
  var readerJump = false
  /// The pull under way has asked already (`readerScrolled`)
  var pulled = false
  /// A look at whether the thread fills its room is on its way (`fillWhenSettled`)
  private var fillChecked = false

  /// A row's height before it is first measured; rows still at it are no reading position (`firstVisibleItem`)
  static let estimatedRowHeight: CGFloat = 120
  /// A drag that ends this close to the end follows it again
  static let followDistance: CGFloat = 120

  private struct ReaderAnchor {
    let indexPath: IndexPath
    let itemID: String?
    let offset: CGFloat
  }

  private var lastLaidOutGeometry: TranscriptViewportGeometry?
  private var lastLaidOutWidth: CGFloat?
  private var lastReaderAnchor: ReaderAnchor?
  private var pendingReaderAnchor: ReaderAnchor?
  private var isRestoringBottomAnchor = false
  private var isPerformingLayout = false
  private var needsInitialBottomPosition = true
  private var bottomGlide: CADisplayLink?
  private var glideVelocity: CGFloat = 0
  /// The row whose end stays where it is through the layout pass in which an earlier part of the session takes its
  /// room above it (`keepEnd`)
  private var heldEnd: VisibleItem?

  /// The room between the insets
  var room: CGFloat { bounds.height - adjustedContentInset.top - adjustedContentInset.bottom }
  /// How far the end lies below the room's bottom edge
  var distanceToEnd: CGFloat { viewportGeometry.bottomOffset - contentOffset.y }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    if window != nil {
      if needsInitialBottomPosition { maintainsBottomAnchor = true }
      if maintainsBottomAnchor { lastLaidOutGeometry = nil }
      setNeedsLayout()
    } else {
      stopBottomGlide()
    }
  }

  /// Follows the latest output. Streamed Markdown lands in multi-line chunks, so snapping reads as a stutter; gliding
  /// moves a few points a frame, re-reading the target each frame as content keeps growing. Distances over a screen
  /// (a reply that lands whole, the jump back from history) snap so the rows between never render
  func followBottom(animated: Bool) {
    maintainsBottomAnchor = true
    guard animated, motion, !UIAccessibility.isReduceMotionEnabled, window != nil,
          abs(viewportGeometry.bottomOffset - contentOffset.y) <= bounds.height else {
      stopBottomGlide()
      snapToBottom()
      return
    }
    guard bottomGlide == nil, abs(contentOffset.y - viewportGeometry.bottomOffset) > 0.5 else { return }
    // CADisplayLink retains its target until invalidated; every stop path invalidates it
    let link = CADisplayLink(target: self, selector: #selector(stepBottomGlide))
    link.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 120, preferred: 120)
    link.add(to: .main, forMode: .common)
    bottomGlide = link
  }

  func stopFollowing() { stopBottomGlide() }

  private func stopBottomGlide() {
    bottomGlide?.invalidate()
    bottomGlide = nil
    glideVelocity = 0
  }

  private func snapToBottom() {
    let target = viewportGeometry.bottomOffset
    guard abs(contentOffset.y - target) > 0.5 else { return }
    isRestoringBottomAnchor = true
    contentOffset.y = target
    isRestoringBottomAnchor = false
  }

  @objc private func stepBottomGlide(_ link: CADisplayLink) {
    guard maintainsBottomAnchor, window != nil, !isTracking, !isDragging, !isDecelerating else {
      stopBottomGlide()
      return
    }
    let target = viewportGeometry.bottomOffset
    let step = TranscriptViewportGeometry.glideStep(from: contentOffset.y, velocity: glideVelocity, to: target,
                                                    elapsed: link.targetTimestamp - link.timestamp)
    // A process can close while the spring still carries forward velocity. Its new end is a hard boundary, not a
    // spring target beyond which the scroll view may bounce (UIKit would clamp it again on the next layout).
    let offset = min(target, max(-adjustedContentInset.top, step.offset))
    glideVelocity = step.velocity
    isRestoringBottomAnchor = true
    contentOffset.y = offset
    isRestoringBottomAnchor = false
    if offset != step.offset || (offset == target && step.velocity == 0) { stopBottomGlide() }
  }

  func releaseReaderAnchor() {
    pendingReaderAnchor = nil
    lastReaderAnchor = nil
  }

  /// The latest fold toggled by hand (`followIfAtEndAfterFold`); an older one's check stands down
  private var foldToggles = 0

  /// A fold toggled by hand let go of the end (`TranscriptControl.hold`). Once it has settled, a thread whose end is in
  /// view again (a fold closed back down to it, or a thread shorter than its room) follows it, as a drag let go there
  /// does; one whose end the fold pushed below the room stays where the reader is
  func followIfAtEndAfterFold() {
    foldToggles += 1
    let toggle = foldToggles
    DispatchQueue.main.asyncAfter(deadline: .now() + Motion.foldSettle) { [weak self] in
      guard let self, toggle == foldToggles, window != nil, !maintainsBottomAnchor,
            !isTracking, !isDragging, !isDecelerating else { return }
      // The fold's last frame may still wait for its layout pass; the end is read from the settled rows. At the end
      // means within a point, as on the desktop (`followsBottom`)
      layoutIfNeeded()
      guard distanceToEnd < 1 else { return }
      maintainsBottomAnchor = true
      setNeedsLayout()
    }
  }

  override func setContentOffset(_ contentOffset: CGPoint, animated: Bool) {
    // UICollectionView also uses this while changing its geometry; those moves belong to the reflow, not to the reader
    if !isRestoringBottomAnchor, !isPerformingLayout, contentOffset != self.contentOffset,
       lastLaidOutWidth == bounds.width, lastLaidOutGeometry == viewportGeometry {
      releaseReaderAnchor()
      maintainsBottomAnchor = false
    }
    super.setContentOffset(contentOffset, animated: animated)
  }

  override func accessibilityScroll(_ direction: UIAccessibilityScrollDirection) -> Bool {
    releaseReaderAnchor()
    maintainsBottomAnchor = false
    readerJump = true
    return super.accessibilityScroll(direction)
  }

  private var viewportGeometry: TranscriptViewportGeometry {
    TranscriptViewportGeometry(contentHeight: contentSize.height, viewportHeight: bounds.height,
                               topInset: adjustedContentInset.top, bottomInset: adjustedContentInset.bottom)
  }

  override func layoutSubviews() {
    defer { if fillEarlier != nil, !fillChecked, short { fillWhenSettled() } }
    let previousOffset = contentOffset.y
    let beforeLayout = viewportGeometry
    // Self-sizing can land before or during super's layout. Keep the following reader's gap when a process closes,
    // rather than keeping the cell's top until UIKit clamps to the shrinking end and abruptly reverses the reply.
    let shrinkOffset = lastLaidOutGeometry.flatMap { previous -> CGFloat? in
      guard maintainsBottomAnchor, !isTracking, !isDragging, !isDecelerating,
            beforeLayout.glidesToBottom(after: previous), beforeLayout.contentHeight < previous.contentHeight else { return nil }
      return previousOffset - (previous.contentHeight - beforeLayout.contentHeight)
    }
    isPerformingLayout = true
    defer { isPerformingLayout = false }
    if window != nil, !safeFrame.isEmpty {
      let frame = convert(bounds, to: nil)
      let scale = traitCollection.displayScale
      // Converting scrolling bounds leaves subpixel noise. Reassigning that as an inset invalidates self-sizing rows
      // on every glide frame even though neither the keyboard nor the dock moved.
      let top = (max(0, safeFrame.minY - frame.minY) * scale).rounded() / scale
      let bottom = (max(0, frame.maxY - safeFrame.maxY) * scale).rounded() / scale
      let inset = UIEdgeInsets(top: top, left: 0, bottom: bottom, right: 0)
      if abs(contentInset.top - top) > 0.5 || abs(contentInset.bottom - bottom) > 0.5 { contentInset = inset }
    }
    // Asking for visible cells after a bounds change can already reflow: a width change goes back to the reading place
    // saved at the end of the previous layout instead
    if maintainsBottomAnchor || isTracking || isDragging || isDecelerating {
      pendingReaderAnchor = nil
    } else if pendingReaderAnchor == nil, let width = lastLaidOutWidth, width != bounds.width {
      pendingReaderAnchor = lastReaderAnchor
    }
    let visibleAnchor = lastLaidOutWidth == bounds.width && !needsInitialBottomPosition ? firstVisibleItem() : nil
    // UIKit animates self-sizing cell changes; streamed text then draws over the next row while frames ease into
    // place. Resize in the frame the content changes; following is the glide's job
    UIView.performWithoutAnimation { super.layoutSubviews() }
    let afterLayout = viewportGeometry
    let shrank = maintainsBottomAnchor && !isTracking && !isDragging && !isDecelerating
      && afterLayout.glidesToBottom(after: beforeLayout) && afterLayout.contentHeight < beforeLayout.contentHeight
    if shrank || shrinkOffset != nil {
      let offset = (shrinkOffset ?? previousOffset) + afterLayout.contentHeight - beforeLayout.contentHeight
      isRestoringBottomAnchor = true
      contentOffset.y = min(afterLayout.bottomOffset, max(-afterLayout.topInset, offset))
      isRestoringBottomAnchor = false
    } else if let heldEnd {
      restoreEnd(heldEnd)
    } else if let visibleAnchor { keepOnScreen(visibleAnchor) }

    let geometry = viewportGeometry
    let dockGlides = dockGlides
    defer {
      self.dockGlides = false
      lastLaidOutGeometry = geometry
      lastLaidOutWidth = bounds.width
      if pendingReaderAnchor == nil, !maintainsBottomAnchor, !isTracking, !isDragging, !isDecelerating {
        lastReaderAnchor = captureReaderAnchor()
      } else if maintainsBottomAnchor {
        lastReaderAnchor = nil
      }
    }

    // Hosted rows can finish measuring on a later pass: the same row stays anchored until the reader takes over
    if let anchor = pendingReaderAnchor {
      let indexPath = anchor.itemID.flatMap { readerIndexPath?($0) } ?? (anchor.itemID == nil ? anchor.indexPath : nil)
      if let indexPath, let attributes = collectionViewLayout.layoutAttributesForItem(at: indexPath) {
        let target = min(geometry.bottomOffset, max(-geometry.topInset, attributes.frame.minY - anchor.offset))
        if abs(contentOffset.y - target) > 0.5 {
          isRestoringBottomAnchor = true
          contentOffset.y = target
          isRestoringBottomAnchor = false
        }
      } else {
        pendingReaderAnchor = nil
      }
    }

    let isInteracting = isTracking || isDragging || isDecelerating || isRestoringBottomAnchor
    if needsInitialBottomPosition, isInteracting { needsInitialBottomPosition = false }
    if needsInitialBottomPosition, window != nil, bounds.height > 0, contentSize.height > 0 {
      needsInitialBottomPosition = false
      maintainsBottomAnchor = true
      isRestoringBottomAnchor = true
      contentOffset = CGPoint(x: contentOffset.x, y: geometry.bottomOffset)
      isRestoringBottomAnchor = false
      return
    }

    guard let bottomY = geometry.restoredBottomOffset(after: lastLaidOutGeometry, maintainsBottomAnchor: maintainsBottomAnchor,
                                                      isInteracting: isInteracting),
          abs(contentOffset.y - bottomY) > 0.5 else { return }
    // Viewport changes (the keyboard, a dock) land with the change itself, but for a dock moving in an animation
    if geometry.glidesToBottom(after: lastLaidOutGeometry) || (dockGlides && geometry.onlyDockMoved(after: lastLaidOutGeometry)),
       lastLaidOutWidth == bounds.width {
      followBottom(animated: true)
      return
    }
    stopBottomGlide()
    isRestoringBottomAnchor = true
    contentOffset = CGPoint(x: contentOffset.x, y: bottomY)
    isRestoringBottomAnchor = false
  }

  struct VisibleItem {
    let itemID: String?
    let indexPath: IndexPath
    let minYInViewport: CGFloat
    let maxYInViewport: CGFloat
  }

  /// The rows are taller than the room: there is somewhere to scroll
  private var scrolls: Bool { contentSize.height > room }

  /// Every row is on screen and measured, and together they leave the room unfilled. Rows not yet measured stand at
  /// the estimate, which makes a few very tall ones read short for a pass
  private var short: Bool {
    guard window != nil, !needsInitialBottomPosition, contentSize.height > 0, !scrolls, numberOfSections > 0 else { return false }
    let cells = visibleCells
    return cells.count == numberOfItems(inSection: 0)
      && cells.allSatisfy { abs($0.frame.height - Self.estimatedRowHeight) > 0.5 }
  }

  /// A thread that leaves its room unfilled has nothing to scroll up with, so it asks for the part ahead by itself.
  /// Decided once the passes of this turn are through: a row that just took in a part still has its old height in the
  /// pass after it, and the thread would ask for one part more than fills the room
  private func fillWhenSettled() {
    fillChecked = true
    DispatchQueue.main.async { [weak self] in
      guard let self else { return }
      fillChecked = false
      layoutIfNeeded()
      if short { fillEarlier?() }
    }
  }

  /// A scroll step. Only the reader's own (a finger, a fling, a jump) with the top of the view within a screen asks
  /// for the part ahead; a part that lands, or any layout change, never asks for the next one. A part can add no
  /// height at all (the steps of a finished turn sit in a closed fold), so a thread that asked whenever the spinner
  /// was in view would pull a huge turn part after part under a reader resting at the top. A fling still moving keeps
  /// stepping, so scrolling up keeps loading. A thread too short to scroll has only the pull at its top: one ask a pull
  func readerScrolled() {
    guard let loadEarlier, window != nil, !isRestoringBottomAnchor, !isPerformingLayout, !needsInitialBottomPosition,
          isTracking || isDragging || isDecelerating || readerJump else { return }
    let above = contentOffset.y + adjustedContentInset.top
    if scrolls {
      if above < room { loadEarlier() }
    } else if above < -0.5, !pulled {
      pulled = true
      loadEarlier()
    }
  }

  /// The first visible row the reader has seen measured, from the cells' last frames (asking the layout here would
  /// apply pending size changes before they can be measured against). A row still at its estimated height is skipped:
  /// scrolling up it grows as it is first measured, and the rows below it are what the reader is looking at
  func firstVisibleItem(skipping skipped: String? = nil) -> VisibleItem? {
    var first: (IndexPath, CGRect)?
    for cell in visibleCells where cell.frame.intersects(bounds) && abs(cell.frame.height - Self.estimatedRowHeight) > 0.5 {
      guard let indexPath = indexPath(for: cell) else { continue }
      if skipped != nil, readerItemID?(indexPath) == skipped { continue }
      if first.map({ indexPath < $0.0 }) ?? true { first = (indexPath, cell.frame) }
    }
    guard let (indexPath, frame) = first else { return nil }
    return VisibleItem(itemID: readerItemID?(indexPath), indexPath: indexPath, minYInViewport: frame.minY - contentOffset.y,
                       maxYInViewport: frame.maxY - contentOffset.y)
  }

  /// An earlier part of the session went in above `item`: its end stays where the reader saw it. Unlike `keepOnScreen`
  /// this also runs under the reader's finger or fling: the rows went in above the viewport, and left alone the thread
  /// would jump down by their height in the middle of the gesture, which carries on from the corrected offset. The row
  /// the view was cut in is measured with its earlier blocks only by a layout pass, where it would grow downwards from
  /// its top: the pass is made here, with the end still held (`layoutSubviews`), so no scroll step comes in between and
  /// the place held is still the reader's
  func keepEnd(_ item: VisibleItem) {
    restoreEnd(item)
    // The end is where the thread follows already; holding a row there would fight the follow
    guard !maintainsBottomAnchor else { return }
    // A jump to the top still running (the status bar) is aimed at the top edge, wherever the rows went, and its next
    // step would carry the reader on to the new top: it ends here. A finger or a fling is left alone
    if !isTracking, !isDragging, !isDecelerating {
      setContentOffset(contentOffset, animated: false)
      readerJump = false
    }
    heldEnd = item
    layoutIfNeeded()
    heldEnd = nil
  }

  private func restoreEnd(_ item: VisibleItem) {
    guard let indexPath = item.itemID.map({ readerIndexPath?($0) }) ?? item.indexPath,
          let attributes = collectionViewLayout.layoutAttributesForItem(at: indexPath) else { return }
    let geometry = viewportGeometry
    let target = min(max(attributes.frame.maxY - item.maxYInViewport, -geometry.topInset), geometry.bottomOffset)
    guard abs(contentOffset.y - target) > 0.5 else { return }
    isRestoringBottomAnchor = true
    contentOffset.y = target
    isRestoringBottomAnchor = false
  }

  /// When a row that starts above the viewport resizes, UIKit shifts the offset by the whole change; for a streaming
  /// reply that grows at its bottom that snaps the visible text by every chunk. Rows the reader can see stay put
  func keepOnScreen(_ item: VisibleItem) {
    // A drag and its deceleration own the offset; restoring a snapshot's old anchor would fight the gesture.
    guard !isTracking, !isDragging, !isDecelerating else { return }
    // A removed row has nothing to keep in place
    guard let indexPath = item.itemID.map({ readerIndexPath?($0) }) ?? item.indexPath,
          let attributes = collectionViewLayout.layoutAttributesForItem(at: indexPath) else { return }
    let geometry = viewportGeometry
    // A closing process can remove more than a viewport. Never restore beyond the new end: UIKit clamps that offset
    // on the next pass, so keeping the old gap made the two restorations bounce the thread back and forth.
    let target = min(max(attributes.frame.minY - item.minYInViewport, -geometry.topInset), geometry.bottomOffset)
    guard abs(contentOffset.y - target) > 0.5 else { return }
    isRestoringBottomAnchor = true
    contentOffset.y = target
    isRestoringBottomAnchor = false
  }

  private func captureReaderAnchor() -> ReaderAnchor? {
    for indexPath in indexPathsForVisibleItems.sorted() {
      guard let cell = cellForItem(at: indexPath), cell.frame.intersects(bounds) else { continue }
      return ReaderAnchor(indexPath: indexPath, itemID: readerItemID?(indexPath), offset: cell.frame.minY - contentOffset.y)
    }
    return nil
  }
}

extension ObservedFlag {
  /// Writes only a change, so observers redraw when the flag flips and not on every scroll
  func set(_ new: Bool) { if value != new { value = new } }
}
#endif
