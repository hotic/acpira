import SwiftUI

// What iOS 26 added that the app draws with, and its stand-in on iOS 18 (the oldest system the app runs on). One build
// serves both: each wrapper asks the running system, so a phone updated to 26 gets Liquid Glass without a new release.
// Views call these, never the iOS 26 API directly, which would not compile against the iOS 18 deployment target.
// Before 26 glass becomes the system's blur material with a hairline edge, and the glass-only touches (merging,
// morphs, edge effects, nav subtitles) are left out.

/// A Liquid Glass variant that also exists before iOS 26: `Glass` itself cannot be named there
public struct GlassLook: Sendable, Equatable {
  enum Variant: Sendable { case regular, clear, identity }
  let variant: Variant
  let isInteractive: Bool

  public static let regular = GlassLook(variant: .regular, isInteractive: false)
  public static let clear = GlassLook(variant: .clear, isInteractive: false)
  /// No glass at all (a piece dropping its surface while staying in place)
  public static let identity = GlassLook(variant: .identity, isInteractive: false)

  public func interactive() -> GlassLook { GlassLook(variant: variant, isInteractive: true) }

  @available(iOS 26, macOS 26, *)
  var glass: Glass {
    let base: Glass = switch variant {
    case .regular: .regular
    case .clear: .clear
    case .identity: .identity
    }
    return isInteractive ? base.interactive() : base
  }

  /// The blur that stands in for the glass before iOS 26: `.clear` is the see-through one, so it gets the thinnest
  var material: Material? {
    switch variant {
    case .regular: .regularMaterial
    case .clear: .ultraThinMaterial
    case .identity: nil
    }
  }
}

/// How a glass piece enters and leaves its container (`GlassEffectTransition`); nothing before iOS 26
public enum GlassMorph: Sendable {
  case identity, materialize

  @available(iOS 26, macOS 26, *)
  var transition: GlassEffectTransition {
    switch self {
    case .identity: .identity
    case .materialize: .materialize
    }
  }
}

/// `GlassEffectContainer`: glass pieces inside melt into each other. Before iOS 26 the pieces stay apart; the stand-in
/// is a plain `ZStack` so every caller's single child keeps one stable parent either way
public struct GlassGroup<Content: View>: View {
  let spacing: CGFloat?
  let content: Content

  public init(spacing: CGFloat? = nil, @ViewBuilder content: () -> Content) {
    self.spacing = spacing
    self.content = content()
  }

  public var body: some View {
    if #available(iOS 26, macOS 26, *) {
      GlassEffectContainer(spacing: spacing) { content }
    } else {
      ZStack { content }
    }
  }
}

public extension View {
  /// `glassEffect(_:in:)`; before iOS 26 the shape filled with the look's material under a hairline edge. The shape
  /// is the hit shape on both, as glass makes it. `.identity` only clears the fill: switching to and from it keeps
  /// the view's structure, so the content keeps its state as it does under glass
  @ViewBuilder
  func glassSurface(_ look: GlassLook, in shape: some Shape) -> some View {
    if #available(iOS 26, macOS 26, *) {
      glassEffect(look.glass, in: shape)
    } else {
      let fill = look.material.map(AnyShapeStyle.init) ?? AnyShapeStyle(Color.clear)
      background(fill, in: shape)
        .overlay(shape.stroke(look.material == nil ? Color.clear : Palette.line, lineWidth: Metrics.hairline))
        .contentShape(shape)
    }
  }

  /// `glassEffectTransition(_:)`; nothing to morph before iOS 26
  @ViewBuilder
  func glassMorph(_ morph: GlassMorph) -> some View {
    if #available(iOS 26, macOS 26, *) {
      glassEffectTransition(morph.transition)
    } else {
      self
    }
  }

  /// The `.glass` / `.glassProminent` button styles; before iOS 26 `LegacyGlassButtonStyle`. `circle` is for a lone
  /// glyph (with `.buttonBorderShape(.circle)` on 26)
  @ViewBuilder
  func glassButtonStyle(prominent: Bool = false, circle: Bool = false) -> some View {
    if #available(iOS 26, macOS 26, *) {
      if prominent { buttonStyle(.glassProminent) } else { buttonStyle(.glass) }
    } else {
      buttonStyle(LegacyGlassButtonStyle(prominent: prominent, circle: circle))
    }
  }

  /// `navigationSubtitle(_:)` came to iOS with 26; before it the title stands alone
  @ViewBuilder
  func navigationSubtitleIfAvailable(_ subtitle: String) -> some View {
    if #available(iOS 26, macOS 26, *) {
      navigationSubtitle(subtitle)
    } else {
      self
    }
  }

  /// `scrollEdgeEffectHidden(_:for:)`: iOS 18 draws no scroll edge effect to hide
  @ViewBuilder
  func scrollEdgeEffectHiddenIfAvailable(for edges: Edge.Set) -> some View {
    if #available(iOS 26, macOS 26, *) {
      scrollEdgeEffectHidden(true, for: edges)
    } else {
      self
    }
  }

  /// `safeAreaBar(edge:content:)`: before iOS 26 a plain safe-area inset, which takes the same room without the
  /// bar's edge effect
  @ViewBuilder
  func safeAreaBarIfAvailable(edge: VerticalEdge, @ViewBuilder content: () -> some View) -> some View {
    if #available(iOS 26, macOS 26, *) {
      safeAreaBar(edge: edge, content: content)
    } else {
      safeAreaInset(edge: edge, content: content)
    }
  }
}

public extension View {
  /// A toolbar item without the bar's shared glass behind it (`sharedBackgroundVisibility(.hidden)`, iOS 26); iOS 18
  /// draws no shared glass. Trap: an `if #available` inside a toolbar builder links in Debug but not in an optimised
  /// build (the SDK exports no opaque type descriptor for `ToolbarContentBuilder.buildLimitedAvailability`), so the
  /// check sits at the view level and each branch adds its own toolbar
  @ViewBuilder
  func bareToolbarItem(placement: ToolbarItemPlacement, shown: Bool = true,
                       @ViewBuilder content: () -> some View) -> some View {
    let item = content()
    if #available(iOS 26, macOS 26, *) {
      toolbar { if shown { ToolbarItem(placement: placement) { item }.sharedBackgroundVisibility(.hidden) } }
    } else {
      toolbar { if shown { ToolbarItem(placement: placement) { item } } }
    }
  }
}

/// The glass buttons' stand-in before iOS 26: the same capsule (or circle), a translucent fill under a hairline edge,
/// or the tint for a prominent one. The system's bordered styles drew a disabled button as a dark block and a
/// rounded rectangle beside the capsules
struct LegacyGlassButtonStyle: ButtonStyle {
  let prominent: Bool
  let circle: Bool
  /// How far a pressed button fades, as the system's bar buttons do
  private static let pressedOpacity = 0.6

  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .padding(.horizontal, circle ? 0 : Metrics.s3)
      .frame(minWidth: Metrics.ctlSm, minHeight: Metrics.ctlSm)
      .background {
        if prominent {
          Capsule().fill(.tint)
        } else {
          Capsule().fill(Palette.active).overlay(Capsule().stroke(Palette.line, lineWidth: Metrics.hairline))
        }
      }
      .contentShape(Capsule())
      .opacity(configuration.isPressed ? Self.pressedOpacity : 1)
  }
}
