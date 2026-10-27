import AcpiraDesign
import SwiftUI

// ui/Overlay.tsx + Popover / DropdownMenu + Panel / Field / Command at phone width: a popup anchored to its trigger
// (side top, aligned to the trigger's start or end, --pop-gap away, kept --pad inside the screen), drawn on the
// popup-surface menu palette, with the option rows and form controls the webview's menus use. The chat screen hosts
// one popup layer; a trigger hands its anchor and content up through a preference, so the content is rebuilt with
// the trigger's own state on every pass (an effort change redraws the open panel in place).

/// One open popup as its trigger publishes it
struct PopupItem {
  var anchor: Anchor<CGRect>
  /// Above (composer menus) or below (header menus) the trigger
  var side: VerticalEdge
  var edge: HorizontalEdge
  var width: CGFloat
  var content: AnyView
  var dismiss: () -> Void
}

nonisolated struct PopupKey: PreferenceKey {
  static var defaultValue: [PopupItem] { [] }
  static func reduce(value: inout [PopupItem], nextValue: () -> [PopupItem]) { value += nextValue() }
}

/// Publishes a popup while `isPresented` holds
struct PopupAnchor<Panel: View>: ViewModifier {
  @Binding var isPresented: Bool
  var side: VerticalEdge
  var edge: HorizontalEdge
  var width: CGFloat
  let panel: () -> Panel

  func body(content: Content) -> some View {
    content.anchorPreference(key: PopupKey.self, value: .bounds) { anchor in
      guard isPresented else { return [] }
      let binding = $isPresented
      return [PopupItem(anchor: anchor, side: side, edge: edge, width: width, content: AnyView(panel()), dismiss: { binding.wrappedValue = false })]
    }
  }
}

extension View {
  /// A popup on this view's `side`, aligned to its `edge`, `width` wide (a --pop-w-* token)
  func popup<Panel: View>(isPresented: Binding<Bool>, side: VerticalEdge = .top, edge: HorizontalEdge = .leading, width: CGFloat,
                          @ViewBuilder panel: @escaping () -> Panel) -> some View {
    modifier(PopupAnchor(isPresented: isPresented, side: side, edge: edge, width: width, panel: panel))
  }

  /// Draws the popups published inside this view; a tap outside the open one closes it
  func popupLayer() -> some View {
    overlayPreferenceValue(PopupKey.self) { items in
      GeometryReader { proxy in
        if let item = items.last {
          let anchor = proxy[item.anchor]
          // Collision padding --pad on every side; the popup never outgrows the room above its anchor
          let width = min(item.width, proxy.size.width - Metrics.pad * 2)
          let x = min(max(item.edge == .leading ? anchor.minX : anchor.maxX - width, Metrics.pad), proxy.size.width - Metrics.pad - width)
          // The room on the popup's side of the anchor, inside the collision padding
          let top = item.side == .top ? Metrics.pad : anchor.maxY + Metrics.popGap
          let bottom = item.side == .top ? anchor.minY - Metrics.popGap : proxy.size.height - Metrics.pad
          ZStack(alignment: .topLeading) {
            Color.clear
              .contentShape(Rectangle())
              .onTapGesture(perform: item.dismiss)
            // The width is the popup's border box, like w-pop-*
            item.content
              .modifier(PopupSurface())
              .frame(width: width)
              .frame(width: width, height: max(0, bottom - top), alignment: item.side == .top ? .bottom : .top)
              .offset(x: x, y: top)
              // No transition: the webview's popups carry no starting / ending style and open and close at once
          }
          .environment(\.dismissPopup, item.dismiss)
        }
      }
    }
  }
}

extension EnvironmentValues {
  /// Closes the popup this view sits in (a family pick, a mode pick)
  @Entry var dismissPopup: () -> Void = {}
}

/// popupClass: popup-surface, rounded-lg, a 1 px border, p-1, shadow-pop
struct PopupSurface: ViewModifier {
  func body(content: Content) -> some View {
    content
      .padding(Metrics.s1)
      .padding(Metrics.hairline)
      .textStyle(TypeScale.text2)
      .foregroundStyle(Palette.fg1)
      .background(RoundedRectangle(cornerRadius: Metrics.rLg).fill(Palette.menuBg))
      .overlay(RoundedRectangle(cornerRadius: Metrics.rLg).strokeBorder(Palette.menuLine))
      .clipShape(RoundedRectangle(cornerRadius: Metrics.rLg))
      // --shadow: 0 8px 28px, 0 1px 2px
      .shadow(color: Palette.shadowWide, radius: Metrics.popShadowRadius, y: Metrics.popShadowY)
      .shadow(color: Palette.shadowContact, radius: Metrics.popContactRadius, y: Metrics.popContactY)
  }
}

// MARK: - Option rows

/// optionClass + OptionContent: a --row tall row, a 14 pt mark, the name (and a faint second line), the check column.
/// Menus carry one fill only, the press highlight; the current row is marked by its check
struct MenuOption<Mark: View>: View {
  var title: String
  var description: String?
  var checked: Bool
  /// Every row keeps the check column once any row is checked
  var checkSlot: Bool
  var action: () -> Void
  let mark: Mark
  @Environment(\.sheetRows) private var sheet

  init(_ title: String, description: String? = nil, checked: Bool, checkSlot: Bool, action: @escaping () -> Void,
       @ViewBuilder mark: () -> Mark) {
    self.title = title; self.description = description; self.checked = checked; self.checkSlot = checkSlot
    self.action = action; self.mark = mark()
  }

  var body: some View {
    // In a bottom sheet the same row grows to a thumb's size: a 20 pt mark, body type, a 52 pt floor
    let mark = sheet ? Metrics.sheetMark : Metrics.icon
    Button(action: action) {
      HStack(spacing: sheet ? Metrics.s3 : Metrics.s2) {
        if !(self.mark is EmptyView) {
          self.mark
            .frame(width: Metrics.icon, height: Metrics.icon)
            .scaleEffect(mark / Metrics.icon)
            .frame(width: mark, height: mark)
        }
        VStack(alignment: .leading, spacing: sheet ? Metrics.s0_5 : 0) {
          Text(web: title, paragraph: false).lineLimit(1).truncationMode(.tail)
            .textStyle(sheet ? TypeScale.field : TypeScale.text2)
          if let description {
            Text(web: description, paragraph: false)
              .textStyle(TypeScale.text3)
              .foregroundStyle(Palette.popFg3)
              .lineLimit(1)
              .truncationMode(.tail)
          }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        if checked {
          Icon(.check, size: sheet ? Metrics.iconCtl : Metrics.icon, strokeWidth: 2)
        } else if checkSlot {
          Color.clear.frame(width: Metrics.icon, height: Metrics.icon)
        }
      }
      // A two-line row drops the row floor for py-1.5
      .padding(.vertical, description == nil || sheet ? 0 : Metrics.s1_5)
      .frame(minHeight: sheet ? Metrics.sheetRow : description == nil ? Metrics.row : 0)
      .padding(.horizontal, sheet ? Metrics.s3 : Metrics.s2)
    }
    .buttonStyle(MenuOptionStyle())
  }
}

extension MenuOption where Mark == EmptyView {
  init(_ title: String, description: String? = nil, checked: Bool, checkSlot: Bool, action: @escaping () -> Void) {
    self.init(title, description: description, checked: checked, checkSlot: checkSlot, action: action) { EmptyView() }
  }
}

/// option-highlighted under a finger: the hover fill while pressed; disabled rows read fg-3
struct MenuOptionStyle: ButtonStyle {
  @Environment(\.isEnabled) private var enabled
  @Environment(\.menuInk) private var ink
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .foregroundStyle(enabled ? ink ?? Palette.fg1 : Palette.popFg3)
      .background(RoundedRectangle(cornerRadius: Metrics.rMd).fill(configuration.isPressed ? Palette.popHover : .clear))
      .contentShape(RoundedRectangle(cornerRadius: Metrics.rMd))
  }
}

/// A popup's option list: scrolls past eight rows, the current row brought to the middle when it opens
struct PopupList<Content: View>: View {
  var current: String?
  let content: Content

  init(current: String? = nil, @ViewBuilder content: () -> Content) {
    self.current = current; self.content = content()
  }

  @Environment(\.sheetRows) private var sheet

  var body: some View {
    if sheet {
      // A sheet scrolls as a whole (PickerSheet), so the list is just its rows
      VStack(spacing: 0) { content }
    } else {
      list
    }
  }

  private var list: some View {
    ScrollViewReader { reader in
      ScrollView(.vertical) {
        VStack(spacing: 0) { content }
      }
      .scrollIndicators(.hidden)
      .frame(maxHeight: Metrics.popMax)
      .fixedSize(horizontal: false, vertical: true)
      .onAppear { if let current { reader.scrollTo(current, anchor: .center) } }
    }
  }
}

/// Command.Input: the search field over a long list, a --ctl bar with a hairline under it
struct PopupSearch: View {
  @Binding var query: String

  var body: some View {
    HStack(spacing: Metrics.s2) {
      Icon(.search, size: Metrics.icon).foregroundStyle(Palette.popFg3)
      TextField(text: $query) { Text(web: t("common.search")).foregroundStyle(Palette.popFg3) }
        .textStyle(TypeScale.text2)
        .foregroundStyle(Palette.fg1)
        .autocorrectionDisabled()
        #if os(iOS)
        .textInputAutocapitalization(.never)
        #endif
    }
    .padding(.horizontal, Metrics.s2)
    .frame(height: Metrics.ctl)
    .overlay(alignment: .bottom) { Rectangle().fill(Palette.menuLine).frame(height: Metrics.hairline) }
    .padding(.bottom, Metrics.s1)
  }
}

/// Command.Empty: one faint row when the search matches nothing
struct PopupEmpty: View {
  var body: some View {
    Text(web: t("common.noMatch"))
      .textStyle(TypeScale.text3)
      .foregroundStyle(Palette.popFg3)
      .padding(.horizontal, Metrics.s2)
      .frame(maxWidth: .infinity, minHeight: Metrics.row, alignment: .leading)
  }
}

// MARK: - Form controls

/// Field.tsx SegmentScale: the label row (with header actions at its end) over one segmented capsule; every level
/// spelled out, the current one raised. Segments size to their labels and share the spare width (flex-auto)
struct SegmentScale: View {
  struct Option: Hashable { var value: String; var label: String; var ultra = false; var disabled = false }
  var label: String
  var options: [Option]
  var value: String
  var onChange: (String) -> Void
  var actions: AnyView?

  var body: some View {
    VStack(alignment: .leading, spacing: Metrics.s1) {
      HStack(spacing: Metrics.s2) {
        Text(web: label, paragraph: false)
          .foregroundStyle(Palette.popFg2)
          .lineLimit(1)
          .frame(maxWidth: .infinity, alignment: .leading)
        if let actions { HStack(spacing: Metrics.s1) { actions } }
      }
      .frame(minHeight: Metrics.ctlSm)
      GrowRow(spacing: Metrics.s0_5) {
        ForEach(options, id: \.value) { o in
          let on = o.value == value
          Button { onChange(o.value) } label: {
            Text(web: o.label, paragraph: false)
              .textStyle(TypeScale.text3)
              .lineLimit(1)
              .padding(.horizontal, Metrics.s1)
              .frame(maxWidth: .infinity)
              .frame(height: Metrics.ctlSm)
              .foregroundStyle(o.ultra ? (on ? Palette.ultra : Palette.popFg3) : on ? Palette.fg1 : Palette.popFg2)
              .background(RoundedRectangle(cornerRadius: Metrics.rSm).fill(on ? (o.ultra ? Palette.ultra.opacity(0.15) : Palette.popActive) : .clear))
              .contentShape(RoundedRectangle(cornerRadius: Metrics.rSm))
          }
          .buttonStyle(BareButtonStyle())
          .disabled(o.disabled)
          .opacity(o.disabled ? 0.5 : 1)
          .accessibilityAddTraits(on ? .isSelected : [])
        }
      }
      .padding(Metrics.s0_5)
      .background(RoundedRectangle(cornerRadius: Metrics.rMd).fill(Palette.popHover))
    }
    .padding(.horizontal, Metrics.s2)
    .padding(.bottom, Metrics.s1)
  }
}

/// `flex: 1 1 auto` in a row: each child starts at its ideal width and the spare width is shared out evenly
/// (an HStack would share the whole row evenly instead); children shrink the same way when the row is short
struct GrowRow: Layout {
  var spacing: CGFloat

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    let ideals = subviews.map { $0.sizeThatFits(.unspecified) }
    let natural = ideals.reduce(0) { $0 + $1.width } + spacing * CGFloat(max(0, subviews.count - 1))
    return CGSize(width: proposal.width ?? natural, height: ideals.map(\.height).max() ?? 0)
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    let ideals = subviews.map { $0.sizeThatFits(.unspecified).width }
    let spare = bounds.width - ideals.reduce(0, +) - spacing * CGFloat(max(0, subviews.count - 1))
    let share = subviews.isEmpty ? 0 : spare / CGFloat(subviews.count)
    var x = bounds.minX
    for (i, subview) in subviews.enumerated() {
      let w = max(0, ideals[i] + share)
      subview.place(at: CGPoint(x: x, y: bounds.minY), anchor: .topLeading, proposal: ProposedViewSize(width: w, height: bounds.height))
      x += w + spacing
    }
  }
}

/// Field.tsx HeaderToggle: a --ctl-sm pressed / unpressed button for a field header (the Fast toggle). Off it is an
/// outlined ghost in fg-3; on it is raised with a stronger ring, its icon in the caller's colour
struct HeaderToggle<Glyph: View>: View {
  var label: String
  var pressed: Bool
  var onChange: (Bool) -> Void
  let glyph: Glyph
  @Environment(\.isEnabled) private var enabled

  init(_ label: String, pressed: Bool, onChange: @escaping (Bool) -> Void, @ViewBuilder glyph: () -> Glyph) {
    self.label = label; self.pressed = pressed; self.onChange = onChange; self.glyph = glyph()
  }

  var body: some View {
    Button { onChange(!pressed) } label: {
      HStack(spacing: Metrics.s1) {
        glyph
        Text(web: label, paragraph: false)
      }
      .textStyle(TypeScale.text3)
      .foregroundStyle(pressed ? Palette.fg1 : Palette.popFg3)
      .padding(.horizontal, Metrics.s1_5)
      .frame(height: Metrics.ctlSm)
      .background(RoundedRectangle(cornerRadius: Metrics.rSm).fill(pressed ? Palette.popActive : .clear))
      .overlay(RoundedRectangle(cornerRadius: Metrics.rSm).strokeBorder(pressed ? Palette.lineStrong : Palette.menuLine))
      .contentShape(RoundedRectangle(cornerRadius: Metrics.rSm))
      .opacity(enabled ? 1 : 0.5)
    }
    .buttonStyle(BareButtonStyle())
    .accessibilityAddTraits(pressed ? .isSelected : [])
  }
}

/// The Fast bolt: outlined, filled in --fast while Fast is on
struct FastGlyph: View {
  var on: Bool
  var size: CGFloat = Metrics.chevron
  var body: some View {
    ZStack {
      if on { IconShape(icon: .zap).fill(Palette.fast) }
      Icon(.zap, size: size, strokeWidth: 2).foregroundStyle(on ? Palette.fast : .primary)
    }
    .frame(width: size, height: size)
  }
}

/// Field.tsx SwitchRow: a --row line with the label and the switch at its end; the whole row toggles
struct SwitchRow: View {
  var label: String
  var checked: Bool
  var onChange: (Bool) -> Void
  @Environment(\.isEnabled) private var enabled

  var body: some View {
    Button { onChange(!checked) } label: {
      HStack(spacing: Metrics.s2) {
        Text(web: label, paragraph: false).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
        Capsule()
          .fill(checked ? Palette.btn1Bg : Palette.popActive)
          .frame(width: Metrics.switchTrackW, height: Metrics.switchTrackH)
          .overlay(alignment: .leading) {
            Circle()
              .fill(checked ? Palette.btn1Fg : Palette.popFg3)
              .frame(width: Metrics.switchThumb, height: Metrics.switchThumb)
              .offset(x: checked ? Metrics.switchOn : Metrics.switchOff)
          }
          .opacity(enabled ? 1 : 0.5)
      }
      .padding(.horizontal, Metrics.s2)
      .frame(minHeight: Metrics.row)
    }
    .buttonStyle(MenuOptionStyle())
    .accessibilityAddTraits(checked ? .isSelected : [])
  }
}

extension EnvironmentValues {
  /// `-popup mode|model`: the composer menu a screenshot opens on launch
  @Entry var initialPopup: String? = nil
  /// `-draft TEXT`: what the chat's composer starts with
  @Entry var initialDraft: String? = nil
  /// A menu row's own ink (delete in --danger), icon and label alike
  @Entry var menuInk: Color? = nil
}
