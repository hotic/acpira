import AcpiraDesign
import AcpiraModels
import SwiftUI
import UniformTypeIdentifiers

// Drag and drop in the by-project list, the desktop list's drags (SessionList.tsx) on the system's drag interaction: a
// long press lifts the row into its menu, and moving the finger turns the lift into a drag (as in Files). A session drops
// on another project (its header or its rows) to move there; a project drops above or below another project, or on a
// section's caption, to take that place, in that section or out of one. Pinned sessions do not drag.
// The system decides between menu and drag: a finger that moves as the row lifts drags it, one that waits for the menu
// and then slides towards it picks menu items instead (observed in the simulator, as in Files).
//
// SwiftUI reports no drag that ends without a drop, so the list's `held` may outlive its drag: zones read it only while
// one hovers them, and a drop acts only when its payload names the held item (another app's text names none).

/// What a drag in the list holds: a session by id, a project by folder
enum ListDragItem: Equatable {
  case session(String)
  case project(String)

  /// The text a drag carries, readable by this process only
  var payload: String {
    switch self {
    case .session(let id): "acpira-list:session:\(id)"
    case .project(let cwd): "acpira-list:project:\(cwd)"
    }
  }

  var provider: NSItemProvider {
    let provider = NSItemProvider()
    let data = Data(payload.utf8)
    provider.registerDataRepresentation(forTypeIdentifier: UTType.utf8PlainText.identifier, visibility: .ownProcess) { done in
      done(data, nil)
      return nil
    }
    return provider
  }
}

/// Where a drop would land
enum ListDropTarget: Equatable {
  /// A session moving into this project
  case project(String)
  /// A project placed in `section` (nil: among the projects in none) before `before` (nil: at that run's end)
  case slot(section: String?, before: String?)
}

/// A row taking drops: `resolve` names the target for the held item at `y` of the row's `height` (nil: no drop here).
/// The target hovered is written to `over`, which the list draws; `perform` runs a drop whose payload checked out
struct ListDropZone: ViewModifier {
  var held: ListDragItem?
  @Binding var over: ListDropTarget?
  var perform: (ListDragItem, ListDropTarget) -> Void
  var resolve: (_ held: ListDragItem, _ y: CGFloat, _ height: CGFloat) -> ListDropTarget?
  @State private var height: CGFloat = 0
  /// The target this zone last marked: leaving clears only that, since the next zone may have marked its own first
  @State private var mine: ListDropTarget?

  func body(content: Content) -> some View {
    content
      .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height = $0 }
      .onDrop(of: [.utf8PlainText], delegate: Delegate(zone: self))
  }

  private struct Delegate: DropDelegate {
    let zone: ListDropZone

    private func target(_ info: DropInfo) -> ListDropTarget? {
      guard let held = zone.held else { return nil }
      return zone.resolve(held, info.location.y, zone.height)
    }

    func validateDrop(info: DropInfo) -> Bool { zone.held != nil && info.hasItemsConforming(to: [.utf8PlainText]) }

    func dropEntered(info: DropInfo) { mark(target(info)) }

    func dropUpdated(info: DropInfo) -> DropProposal? {
      let next = target(info)
      mark(next)
      return DropProposal(operation: next == nil ? .forbidden : .move)
    }

    func dropExited(info: DropInfo) { mark(nil) }

    func performDrop(info: DropInfo) -> Bool {
      let at = target(info)
      zone.over = nil
      zone.mine = nil
      guard let held = zone.held, let at, let provider = info.itemProviders(for: [.utf8PlainText]).first else { return false }
      // Read back on the provider's queue, acted on on the main actor
      nonisolated(unsafe) let perform = zone.perform
      _ = provider.loadDataRepresentation(for: .utf8PlainText) { data, _ in
        guard let data, String(decoding: data, as: UTF8.self) == held.payload else { return }
        Task { @MainActor in
          // The lift was the system's (the long press); the landing is the list's own
          Haptics.play(.drop)
          perform(held, at)
        }
      }
      return true
    }

    /// Marks `next`, or with none takes away this zone's own mark only
    private func mark(_ next: ListDropTarget?) {
      if let next {
        if zone.over != next { zone.over = next }
        zone.mine = next
      } else if let mine = zone.mine {
        if zone.over == mine { zone.over = nil }
        zone.mine = nil
      }
    }
  }
}

extension View {
  /// Lets the row lift into a drag of `item` (none: the row does not drag), shown as `preview`
  @ViewBuilder
  func listDrag<Preview: View>(_ item: ListDragItem?, onStart: @escaping (ListDragItem) -> Void,
                               @ViewBuilder preview: () -> Preview) -> some View {
    if let item {
      onDrag({ onStart(item); return item.provider }, preview: preview)
    } else {
      self
    }
  }

  /// The drop marks of a list row: a fill while a session would move into it, a line on the edge a project would land
  /// against (SessionList.tsx FRAME / LINE). Overlays, so nothing in the list moves
  func dropMarks(hot: Bool, insert: VerticalEdge?) -> some View {
    background { Rectangle().fill(hot ? Palette.active : .clear).animation(Motion.ease, value: hot) }
      .overlay(alignment: insert == .bottom ? .bottom : .top) {
        if insert != nil {
          Capsule().fill(Palette.fg1.opacity(0.7))
            .frame(height: Metrics.s0_5)
            .padding(.horizontal, Metrics.pad)
            .offset(y: insert == .bottom ? Metrics.s0_5 / 2 : -Metrics.s0_5 / 2)
            .allowsHitTesting(false)
        }
      }
  }
}

/// The lifted row under the finger: its mark and name on a card, rather than a full-width row of loose text
struct ListDragPreview<Lead: View>: View {
  let title: String
  @ViewBuilder var lead: () -> Lead

  var body: some View {
    HStack(spacing: Metrics.s2) {
      lead().foregroundStyle(Palette.fg2).frame(width: Metrics.rowMark, height: Metrics.rowMark)
      Text(web: title, paragraph: false).lineLimit(1).truncationMode(.tail)
        .textStyle(TypeScale.field, weight: .medium)
        .foregroundStyle(Palette.fgStrong)
        // The card hugs a short name; a long one stops here and truncates
        .frame(maxWidth: Metrics.dragPreview, alignment: .leading)
    }
    .padding(.horizontal, Metrics.gap)
    .frame(height: Metrics.ctl)
    .fixedSize()
    .background(Palette.bg2, in: RoundedRectangle(cornerRadius: Metrics.rLg, style: .continuous))
  }
}
