#if os(iOS)
import AcpiraModels
import SwiftUI
import Testing
import UIKit
@testable import AcpiraUI

@MainActor @Suite(.serialized) struct TranscriptCollectionTests {
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

  private final class SingleRow: NSObject, UICollectionViewDataSource {
    func collectionView(_ collectionView: UICollectionView, numberOfItemsInSection section: Int) -> Int { 1 }
    func collectionView(_ collectionView: UICollectionView, cellForItemAt indexPath: IndexPath) -> UICollectionViewCell {
      collectionView.dequeueReusableCell(withReuseIdentifier: "row", for: indexPath)
    }
  }
}
#endif
