import AcpiraDesign
import SwiftUI

// The phone's floating glass controls: lone circles and groups sharing one capsule, over content that scrolls under them.

/// A floating glass control: a circle holding one glyph
struct GlassIconButton: View {
  let icon: LucideIcon
  let label: String
  var action: () -> Void

  var body: some View {
    Button(action: action) {
      Icon(icon, size: Metrics.glassIcon)
        .foregroundStyle(Palette.fg1)
        .frame(width: Metrics.glassCtl, height: Metrics.glassCtl)
        .contentShape(Capsule())
    }
    .buttonStyle(.plain)
    .glassEffect(GlassStyle.control.interactive(), in: Capsule())
    .accessibilityLabel(Text(web: label))
  }
}
