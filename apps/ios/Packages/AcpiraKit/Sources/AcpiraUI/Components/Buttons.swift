import AcpiraDesign
import SwiftUI

// ui/Button.tsx: two neutral button tiers, the square icon button, the toolbar Chip and its ChipTag badge.

public enum ButtonVariant { case primary, secondary }

/// Button: --ctl tall, --r-md, 12 pt sides, text-2. Primary is a tonal fill with strong text and a firmer outline,
/// secondary an outline that fills when pressed
public struct AcpButtonStyle: ButtonStyle {
  var variant: ButtonVariant
  var compact: Bool
  /// The plain `h-ctl` button (a menu trigger): one line, exactly --ctl tall. Without it the button is the answer
  /// form (`h-auto min-h-ctl py-gap-half`), whose label may wrap
  var fixed: Bool
  public init(_ variant: ButtonVariant = .secondary, compact: Bool = false, fixed: Bool = false) {
    self.variant = variant; self.compact = compact; self.fixed = fixed
  }

  public func makeBody(configuration: Configuration) -> some View {
    let primary = variant == .primary
    let pressed = configuration.isPressed
    HStack(spacing: Metrics.s1_5) { configuration.label }
      .textStyle(TypeScale.text2, weight: primary ? .medium : .regular)
      .foregroundStyle(primary ? Palette.fgStrong : pressed ? Palette.fg1 : Palette.fg2)
      .fixedSize(horizontal: fixed, vertical: true)
      // px inside a 1 px border
      .padding(.horizontal, (compact ? Metrics.s2 : Metrics.s3) + Metrics.hairline)
      // h-auto min-h-ctl py-gap-half: the label's line box, the half gap and the hairline on each side
      .padding(.vertical, compact || fixed ? 0 : Metrics.gapHalf + Metrics.hairline)
      .frame(minHeight: compact ? Metrics.ctlSm : Metrics.ctl, maxHeight: fixed ? Metrics.ctl : nil)
      // A capsule, like every other control on the phone
      .background(Capsule().fill(primary ? (pressed ? Palette.chipHover : Palette.active) : (pressed ? Palette.hover : .clear)))
      .overlay(Capsule().stroke(primary ? Palette.lineStrong : Palette.line, lineWidth: Metrics.hairline))
      .contentShape(Capsule())
  }
}

/// `web`: the webview's sm (--ctl-sm with a 14 pt icon at the airy density), for buttons inside the transcript, which
/// is matched against the webview rather than drawn at the phone's control sizes
public enum IconButtonSize { case regular, sm, web }

/// Square icon button: --ctl with a 16 pt icon, or --ctl-sm with a 14 pt icon; fg-2, pressed gets the hover fill
public struct IconButton: View {
  let icon: LucideIcon
  var size: IconButtonSize
  var label: String
  var open: Bool
  let action: () -> Void

  public init(_ icon: LucideIcon, size: IconButtonSize = .regular, label: String, open: Bool = false, action: @escaping () -> Void) {
    self.icon = icon; self.size = size; self.label = label; self.open = open; self.action = action
  }

  public var body: some View {
    Button(action: action) {
      Icon(icon, size: size == .web ? WebMetrics.icon : size == .sm ? Metrics.icon : Metrics.iconCtl)
    }
    .buttonStyle(IconButtonStyle(side: size == .web ? WebMetrics.ctlSm : size == .sm ? Metrics.ctlSm : Metrics.ctl, open: open))
    .accessibilityLabel(Text(web: label))
  }
}

struct IconButtonStyle: ButtonStyle {
  var side: CGFloat
  var open = false
  /// The resting ink when it is not the context's fg-2 (a popup bar's fg-3); pressed, the popup's own tonal fills
  var ink: Color?
  @Environment(\.composerInk) private var composerInk
  func makeBody(configuration: Configuration) -> some View {
    let lit = configuration.isPressed || open
    configuration.label
      .foregroundStyle(lit ? Palette.fg1 : ink ?? (composerInk ? Palette.cmpFg2 : Palette.fg2))
      .frame(width: side, height: side)
      .background(Capsule().fill(open ? (ink == nil ? Palette.active : Palette.popActive)
        : configuration.isPressed ? (ink == nil ? Palette.hover : Palette.popHover) : .clear))
      .contentShape(Capsule())
  }
}

public enum ChipVariant { case quiet, solid }

/// The chip's contents: an optional lead mark, the label with faint meta, badges, and the caret
public struct ChipLabel<Mark: View, Tags: View>: View {
  var variant: ChipVariant
  var text: String
  var meta: String?
  var caret: Bool
  let mark: Mark
  let tags: Tags

  public init(_ text: String, variant: ChipVariant = .quiet, meta: String? = nil, caret: Bool = true,
              @ViewBuilder mark: () -> Mark, @ViewBuilder tags: () -> Tags) {
    self.text = text; self.variant = variant; self.meta = meta; self.caret = caret; self.mark = mark(); self.tags = tags()
  }

  public var body: some View {
    if !(mark is EmptyView) {
      mark.frame(width: Metrics.icon, height: Metrics.icon)
        .foregroundStyle(variant == .quiet ? Palette.fg3 : Palette.fg1)
    }
    if let meta {
      Text("\(Text(web: text, paragraph: false)) \(Text(web: meta).foregroundStyle(Palette.fg3))").truncationMode(.tail)
    } else {
      Text(web: text).truncationMode(.tail)
    }
    if !(tags is EmptyView) {
      HStack(spacing: Metrics.s0_5) { tags }.fixedSize()
    }
    if caret {
      Icon(.chevronDown, size: Metrics.chevron, strokeWidth: 1.75).foregroundStyle(Palette.fg3)
    }
  }
}

extension ChipLabel where Tags == EmptyView {
  public init(_ text: String, variant: ChipVariant = .quiet, meta: String? = nil, caret: Bool = true, @ViewBuilder mark: () -> Mark) {
    self.init(text, variant: variant, meta: meta, caret: caret, mark: mark, tags: { EmptyView() })
  }
}

extension ChipLabel where Mark == EmptyView, Tags == EmptyView {
  public init(_ text: String, variant: ChipVariant = .quiet, meta: String? = nil, caret: Bool = true) {
    self.init(text, variant: variant, meta: meta, caret: caret, mark: { EmptyView() }, tags: { EmptyView() })
  }
}

extension Button where Label == Text {
  /// A button titled as the webview sets text (CJK at Chrome's PingFang width)
  public init(web title: String, action: @escaping () -> Void) {
    self.init(action: action) { Text(web: title) }
  }
}
