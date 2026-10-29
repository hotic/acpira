import AcpiraDesign
import SwiftUI

// ui/Row.tsx: the one shared row every thought / plan / tool / status / session item grows on. Lead slot (20) with a
// 14 pt icon or the Orb, content on a baseline with an 8 pt gap, trailing meta right-aligned in caption type.
// `process-rows` (ProcessHistory) shrinks row and dense rows to --process-row; `action-row` dims the whole row.

extension EnvironmentValues {
  /// --row inside the current scope (process rows set --process-row)
  @Entry var rowHeight: CGFloat = WebMetrics.row
  /// --row-dense inside the current scope
  @Entry var rowDenseHeight: CGFloat = WebMetrics.rowDense
  /// The motion axis: `false` is the webview's motion = none (shimmer becomes plain fg-2, entrances off)
  @Entry public var motionEnabled: Bool = true
  /// Inside an `action-row`: --fg-2 is the action ink and targets inherit it
  @Entry var actionTone: Bool = false
  /// Inside `composer-field`: --fg-2 is the field's own mix (`Palette.cmpFg2`)
  @Entry var composerInk: Bool = false
  /// RowEntranceContext: rows that mount while this is true play the `process-row-enter` entrance
  @Entry var rowEnter: Bool = false
  /// --row-enter-delay: 0, or --row-enter-file under `tool-results`
  @Entry var rowEnterDelay: Double = 0
  /// EntranceScopeContext: whether the turn is live, and its name
  @Entry var entranceScope = EntranceScope(live: false, scope: "")
  /// Whether the reader is moving the thread (`Thread`); outside a scrolling thread it never is
  @Entry var scrollGesture = ScrollGesture()
}

/// The reader is dragging the thread or it is still coasting from a fling. A lazy stack builds a row that arrived off
/// screen only once it is scrolled to, so a row mounting now came into view by the scroll, not by arriving, and skips
/// the entrance (the text beside it has none, and the rows alone fading under the finger read as a glitch). A class, so
/// the thread flips it on every scroll phase without redrawing the rows that read it
final class ScrollGesture: @unchecked Sendable {
  var active = false
}

// MARK: - Entrance

/// The live turn and its name; agents may number tool calls per session, so ids alone repeat across turns
struct EntranceScope: Equatable {
  var live: Bool
  var scope: String
}

/// Identities whose rows already entered. A tool changes view shape as it progresses (a running read is a single
/// call, a finished one joins a read group), and a remounted transcript (another session and back, a lazy stack
/// scrolling the turn back in) must not replay the entrance
enum EntranceLedger {
  static var entered = Set<String>()
  static let cap = 4000

  /// Claims `key`; true the first time it is seen
  static func claim(_ key: String) -> Bool {
    let fresh = !entered.contains(key)
    if entered.count >= cap { entered.removeAll() }
    entered.insert(key)
    return fresh
  }
}

/// One claim per mounted EntranceOnce. A class so that resolving it while the body is evaluated (where the
/// environment is readable) and settling it afterwards do not invalidate the view
final class EntranceClaim {
  private var fresh: Bool?
  private var settled = false

  func enter(id: String, scope: EntranceScope) -> Bool {
    if fresh == nil { fresh = EntranceLedger.claim("\(scope.scope)\u{0}\(id)") && scope.live }
    return fresh == true && !settled
  }

  func settle() { settled = true }
}

/// Row.tsx EntranceOnce: rows under `id` enter only with the first mount of that identity in a live turn. Rows
/// that mount later under the same instance (a branch that changes shape as the tool progresses) stay still;
/// content that genuinely appears later gets its own EntranceOnce
struct EntranceOnce<Content: View>: View {
  let id: String
  let content: Content
  @Environment(\.entranceScope) private var scope
  @State private var claim = EntranceClaim()

  init(id: String, @ViewBuilder content: () -> Content) {
    self.id = id
    self.content = content()
  }

  var body: some View {
    content
      .environment(\.rowEnter, claim.enter(id: id, scope: scope))
      // Rows read the flag once, at their own mount: the ones mounted with this pass keep their entrance
      .task { claim.settle() }
  }
}

/// One slot of `process-row-enter`: opacity 0 → 1 over --row-enter-duration, ease-out, after `delay`. The decision
/// is taken at mount, so a row that mounted still never fades later and a fading one finishes its fade. A row mounted
/// by the reader's scroll stays still (`ScrollGesture`)
struct RowEnterFade: ViewModifier {
  var delay: Double
  @Environment(\.rowEnter) private var enter
  @Environment(\.motionEnabled) private var motion
  @Environment(\.scrollGesture) private var gesture
  @State private var shown: Bool?

  func body(content: Content) -> some View {
    content
      .opacity(shown ?? !plays ? 1 : 0)
      .onAppear {
        guard shown == nil else { return }
        if plays {
          withAnimation(.easeOut(duration: Motion.rowEnter).delay(delay)) { shown = true }
        } else {
          shown = true
        }
      }
  }

  private var plays: Bool { enter && motion && !gesture.active }
}

extension View {
  func rowEnterFade(_ delay: Double) -> some View { modifier(RowEnterFade(delay: delay)) }
}

public enum RowTone { case plain, action }

public struct Row<Lead: View, Content: View, Trailing: View>: View {
  @Environment(\.rowHeight) private var rowHeight
  @Environment(\.rowDenseHeight) private var rowDenseHeight
  @Environment(\.rowEnterDelay) private var enterDelay
  var tone: RowTone
  var dense: Bool
  var leadTop: Bool
  let lead: Lead
  let content: Content
  let trailing: Trailing

  public init(tone: RowTone = .plain, dense: Bool = false, leadTop: Bool = false,
              @ViewBuilder lead: () -> Lead, @ViewBuilder content: () -> Content, @ViewBuilder trailing: () -> Trailing) {
    self.tone = tone; self.dense = dense; self.leadTop = leadTop
    self.lead = lead(); self.content = content(); self.trailing = trailing()
  }

  public var body: some View {
    HStack(alignment: leadTop ? .top : .center, spacing: Metrics.gap) {
      // Group(subviews:) drops a slot whose view renders nothing, so an empty lead / trailing takes no gap
      Group(subviews: lead) { parts in
        if !parts.isEmpty {
          ZStack { parts }
            .frame(width: WebMetrics.lead, height: WebMetrics.lead)
            // The slot's contents centre on an icon-wide column at the row's start, so an icon's ink stands on the
            // text edge above and below it rather than three points in; smaller marks centre on that column and
            // larger ones overhang it evenly. The slot keeps its width, so the label column stays put
            .padding(.trailing, (WebMetrics.lead - WebMetrics.icon) / 2)
            .padding(.leading, -(WebMetrics.lead - WebMetrics.icon) / 2)
            .foregroundStyle(tone == .action ? Palette.actionFg : Palette.fg3)
            // lead-top: centre the slot on the first text line
            // The icon here is where a connected rail starts and ends
            .railLeadSlot()
            .padding(.top, leadTop ? (WebType.text2.lineHeight - WebMetrics.lead) / 2 : 0)
            .rowEnterFade(enterDelay)
        }
      }
      // The entrance staggers the content: its first child one --row-enter-part after the lead, the rest two
      Group(subviews: content) { parts in
        HStack(alignment: .firstTextBaseline, spacing: Metrics.s2) {
          ForEach(Array(parts.enumerated()), id: \.element.id) { i, part in
            part.rowEnterFade(enterDelay + Motion.rowEnterPart * (i == 0 ? 1 : 2))
          }
        }
      }
        .frame(maxWidth: .infinity, alignment: .leading)
      Group(subviews: trailing) { parts in
        if !parts.isEmpty {
          HStack(spacing: Metrics.s2) { parts }
            .textStyle(WebType.text3)
            .monospacedDigit()
            .foregroundStyle(tone == .action ? Palette.actionFg : Palette.fg3)
            .fixedSize()
            .rowEnterFade(enterDelay + Motion.rowEnterPart * 2)
        }
      }
    }
    .textStyle(WebType.text2)
    .foregroundStyle(tone == .action ? Palette.actionFg : Palette.fg2)
    // items-start under lead-top: a single line keeps to the top of the row, not its centre
    .frame(minHeight: dense ? rowDenseHeight : rowHeight, alignment: leadTop ? .top : .center)
    .environment(\.actionTone, tone == .action)
    .contentShape(Rectangle())
    // A result rail ends at the bottom of the row its last icon leads
    .transformAnchorPreference(key: RailLeadKey.self, value: .bounds) { leads, row in
      for i in leads.indices where leads[i].row == nil { leads[i].row = row }
    }
  }
}

extension Row where Lead == EmptyView {
  public init(tone: RowTone = .plain, dense: Bool = false,
              @ViewBuilder content: () -> Content, @ViewBuilder trailing: () -> Trailing) {
    self.init(tone: tone, dense: dense, lead: { EmptyView() }, content: content, trailing: trailing)
  }
}

extension Row where Trailing == EmptyView {
  public init(tone: RowTone = .plain, dense: Bool = false, leadTop: Bool = false,
              @ViewBuilder lead: () -> Lead, @ViewBuilder content: () -> Content) {
    self.init(tone: tone, dense: dense, leadTop: leadTop, lead: lead, content: content, trailing: { EmptyView() })
  }
}

extension Row where Lead == EmptyView, Trailing == EmptyView {
  public init(tone: RowTone = .plain, dense: Bool = false, @ViewBuilder content: () -> Content) {
    self.init(tone: tone, dense: dense, lead: { EmptyView() }, content: content, trailing: { EmptyView() })
  }
}

/// A short label that keeps its width; `shimmer` marks the running verb
public struct RowLabel: View {
  let text: String
  var shimmer: Bool
  public init(_ text: String, shimmer: Bool = false) { self.text = text; self.shimmer = shimmer }
  public var body: some View {
    Shimmer(text, active: shimmer).lineLimit(1).fixedSize().cssBaseline()
  }
}

/// A row's target (file / command): one step brighter than the verb, the first thing to truncate
public struct RowTarget: View {
  @Environment(\.actionTone) private var action
  let text: String
  var mono: Bool
  /// An explicit ink (`text-fg-2`, `text-fg-3`); `nil` is the default fg-1 at 85 %
  var ink: Color?
  public init(_ text: String, mono: Bool = false, ink: Color? = nil) { self.text = text; self.mono = mono; self.ink = ink }
  public var body: some View {
    Text(web: text)
      .inlineFont(mono ? WebType.mono : WebType.text2)
      .foregroundStyle(action ? Palette.actionFg : (ink ?? Palette.fg1.opacity(0.85)))
      .lineLimit(1)
      .truncationMode(.tail)
      .cssBaseline()
      .layoutPriority(-1)
  }
}

/// A faint note beside a target: a line range, a hit's line
public struct Aside: View {
  let text: String
  public init(_ text: String) { self.text = text }
  public var body: some View {
    // cssBaseline inside textStyle reads the text-3 line box
    Text(web: text).cssBaseline().textStyle(WebType.text3).monospacedDigit()
      .foregroundStyle(Palette.fg3.opacity(0.7)).lineLimit(1).fixedSize()
  }
}

/// +add in the ok colour, −del in danger
public struct DiffStatView: View {
  let add: Int, del: Int
  public init(add: Int, del: Int) { self.add = add; self.del = del }
  public var body: some View {
    let plus = Text(web: "+\(add)").foregroundStyle(Palette.ok)
    let minus = Text(web: "−\(del)").foregroundStyle(Palette.danger)
    Text("\(plus) \(minus)").monospacedDigit()
  }
}

/// The tappable row look: the hit area reaches `--row-hit-outset` past the text column, pressed rows get the hover fill
public struct RowButtonStyle: ButtonStyle {
  public init() {}
  public func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .padding(.horizontal, Metrics.hit)
      .background(RoundedRectangle(cornerRadius: Metrics.rMd).fill(configuration.isPressed ? Palette.hover : .clear))
      .padding(.horizontal, -Metrics.hit)
      .contentShape(Rectangle())
  }
}
